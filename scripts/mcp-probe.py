#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.12"
# dependencies = ["jsonschema>=4.25"]
# ///
"""Drive every tool of `satz mcp` over raw JSON-RPC and check every byte it writes.

    uv run scripts/mcp-probe.py                          # offline, every group
    uv run scripts/mcp-probe.py --groups read            # what read allows
    uv run scripts/mcp-probe.py --tools satz_triage,satz_report_compliance
    uv run scripts/mcp-probe.py --mode net               # + check_presets, a real Checkov
    uv run scripts/mcp-probe.py --mode live --live-dir <estate-checkout> \\
        --live-estate <main>.satz                        # + ADC: reads the organisation

The probe reads the server's stdout pipe itself, not through an MCP SDK: an SDK drops
a line that is not JSON, so a test built on one cannot see a leak. Any stdout line that
is not a JSON-RPC message fails the run, charged to the call in flight.

Per call: `structuredContent` validates against the tool's `outputSchema` and equals
the JSON in the text block; a refusal is an `isError` result with prose; a missing or
mistyped argument is refused (the form, `isError` or `invalid_params`, is reported);
the duration; the size in bytes and in tokens estimated at four bytes each. The report
ends with each tool's LARGEST result, the number a client's output limit is set from.

Modes, each including the one before:
  offline  no credentials (the ADC path names a file that does not exist), no network;
           Checkov is a stand-in on PATH that prints a fixed report
  net      + satz_check_presets against the upstream library, and a real Checkov
           (`checkov`, else `uvx checkov`)
  live     + gcloud's application-default credentials, over a COPY of --live-dir:
           whoami online, report_compliance live, adopt without execute. Nothing is
           written to the organisation

Servers: `main` (the selected groups), `gp` (get/merge-presets into an estate with no
library), `ceiling` (--allow read), `gated` (--self-gated), `identity` (estates opened
in turn), and `live`. The staged root holds presets/, tests/smoke/, tests/schemas/ and
a pristine copy of presets/, so `presets_dir` sits inside --root.
Exit status 0 when every check passed and stdout carried nothing but JSON-RPC.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

import jsonschema

REPO = Path(__file__).resolve().parent.parent
PROTOCOL = "2025-06-18"
FRAMEWORK = "cis-gcp-5.0"
MODES = ("offline", "net", "live")
GROUPS = ("read", "write", "exec")
EXPECTED_TOOLS = {
    *("satz_open", "satz_estates", "satz_restrict", "satz_require", "satz_questions"),
    *("satz_interview", "satz_triage", "satz_prowler", "satz_transpile_check"),
    *("satz_transpile", "satz_fmt", "satz_review_pack", "satz_check_presets"),
    *("satz_get_presets", "satz_merge_presets", "satz_remediation_items"),
    *("satz_remediation_annotate", "satz_adopt", "satz_update_prerequisites"),
    *("satz_scan_checkov", "satz_report_compliance", "satz_whoami"),
}

# MCP 2025-06-18 requires an object `outputSchema` (`type: "object"`) and an object
# `structuredContent`. A deviation listed here is reported as KNOWN instead of failing;
# one listed that no longer occurs FAILS, so the entry leaves with the fix.
KNOWN_DEVIATIONS = {
    ("satz_triage", "outputSchema"): "the rows are published as a JSON array",
    ("satz_triage", "structuredContent"): "the rows are returned as a JSON array",
}

OUTSIDE = "outside the server's root"
NO_IDENTITY = "satz cannot tell which identity this estate runs as"
SMOKE = {"config": "tests/smoke/config.toml", "estate": "smoke.satz"}
PROWLER = "tests/smoke/prowler.json"
R, W, X = {"read"}, {"read", "write"}, {"read", "write", "exec"}


class Run:
    def __init__(self, groups: set[str], mode: str, tools: set[str]):
        self.groups, self.mode, self.tools = groups, mode, tools
        self.results: list[dict] = []
        self.largest: dict[str, dict] = {}
        self.deviations: set[tuple[str, str]] = set()

    def wants(self, tool: str, needs: set[str], mode: str = "offline") -> bool:
        return (
            (not self.tools or tool in self.tools)
            and needs <= self.groups
            and MODES.index(mode) <= MODES.index(self.mode)
        )

    def record(self, server: str, case: str, status: str, detail="", **extra) -> None:
        self.results.append(
            {"server": server, "case": case, "status": status, "detail": detail} | extra
        )
        ms = f"{extra['ms']:6d}ms" if "ms" in extra else " " * 8
        print(f"[{status:4}] {server:8} {case:56} {ms}  {detail}"[:400], flush=True)

    def check(self, server: str, case: str, ok: bool, detail: str = "") -> None:
        self.record(server, case, "PASS" if ok else "FAIL", "" if ok else detail[:300])

    def deviation(self, tool: str, kind: str, broken: bool, why: str) -> str | None:
        """A protocol deviation: a problem unless KNOWN_DEVIATIONS lists it."""
        if not broken:
            return None
        self.deviations.add((tool, kind))
        return None if (tool, kind) in KNOWN_DEVIATIONS else f"MCP {PROTOCOL}: {why}"


class Server:
    """One `satz mcp` child. A reader thread splits stdout into JSON-RPC messages and
    STRAY lines; a stray line is charged to the request in flight."""

    def __init__(self, run: Run, label: str, satz: Path, root: Path, allow: str,
                 work: Path, env: dict, *flags: str):  # fmt: skip
        self.run, self.label = run, label
        self.stderr = (work / f"stderr-{label}.log").open("wb")
        self.proc = subprocess.Popen(
            [str(satz), "mcp", "--root", str(root), "--allow", allow, *flags],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=self.stderr,
            cwd=root,
            env=env,
        )
        self.q: queue.Queue = queue.Queue()
        self.stray: list[tuple[str, str]] = []
        self.inflight = "startup"
        self.next_id = 0
        self.tools: dict[str, dict] = {}
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        self._handshake()

    def _read(self) -> None:
        assert self.proc.stdout is not None
        for raw in self.proc.stdout:
            line = raw.removesuffix(b"\n")
            try:
                msg = json.loads(line)
            except ValueError:
                msg = None
            if isinstance(msg, dict) and msg.get("jsonrpc") == "2.0":
                self.q.put(msg)
            else:
                text = line.decode("utf-8", "replace")[:400] or "[an empty line]"
                self.stray.append((self.inflight, text))
        self.q.put(None)

    def send(self, msg: dict) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.write((json.dumps(msg) + "\n").encode())
        self.proc.stdin.flush()

    def request(self, method: str, params: dict, timeout: float = 120.0) -> dict:
        self.next_id += 1
        self.inflight = f"{method} {params.get('name', '')}".strip()
        self.send(
            {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        )
        deadline = time.monotonic() + timeout
        while (left := deadline - time.monotonic()) > 0:
            try:
                msg = self.q.get(timeout=left)
            except queue.Empty:
                break
            if msg is None:
                raise RuntimeError(f"the server closed stdout during {self.inflight}")
            if msg.get("id") == self.next_id:
                return msg
        raise TimeoutError(f"no reply to {self.inflight} in {timeout:.0f}s")

    def _handshake(self) -> None:
        hello = {"name": "mcp-probe", "version": "1"}
        r = self.request(
            "initialize",
            {"protocolVersion": PROTOCOL, "capabilities": {}, "clientInfo": hello},
        )
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})
        got = r.get("result", {}).get("protocolVersion")
        self.run.check(
            self.label, f"initialize answers {PROTOCOL}", got == PROTOCOL, str(r)
        )
        self.tools = {
            t["name"]: t for t in self.request("tools/list", {})["result"]["tools"]
        }

    def call(self, case: str, tool: str, args: dict, expect: str = "ok",
             contains: str | None = None, needs: set[str] = R, mode: str = "offline",
             timeout: float = 120.0, setup: bool = False) -> dict:  # fmt: skip
        """Call a tool and check the answer; return its structuredContent (or {}).
        `expect`: ok | error (an isError result) | argerror (invalid_params, or an
        isError result — the form is reported). A `setup` call runs whatever --tools
        and --groups select, because the cases after it depend on it."""
        run = self.run
        if not setup and not run.wants(tool, needs, mode):
            return {}
        before, t0 = len(self.stray), time.monotonic()
        try:
            msg = self.request("tools/call", {"name": tool, "arguments": args}, timeout)
        except (
            TimeoutError,
            RuntimeError,
        ) as e:  # a hung or dead server is the finding
            run.record(self.label, case, "FAIL", str(e), tool=tool)
            return {}
        extra: dict = {"tool": tool, "ms": int((time.monotonic() - t0) * 1000)}
        problems = [f"stray stdout: {line!r}" for _, line in self.stray[before:]]
        result = msg.get("result") or {}
        if "error" in msg:
            if expect == "argerror" and msg["error"].get("code") == -32602:
                extra["refused_as"] = "invalid_params"
            else:
                problems.append(f"JSON-RPC error {msg['error']}")
        else:
            problems += self._judge(case, tool, result, expect, contains, extra)
        status = "FAIL" if problems else "PASS"
        run.record(
            self.label,
            case,
            status,
            "; ".join(problems) or extra.get("refused_as", ""),
            **extra,
        )
        return result.get("structuredContent") or {}

    def _judge(self, case, tool, result, expect, contains, extra) -> list[str]:
        problems: list[str] = []
        texts = [
            c.get("text", "")
            for c in result.get("content", [])
            if c.get("type") == "text"
        ]
        text = "\n".join(texts)
        size = len(json.dumps(result, ensure_ascii=False).encode())
        extra.update(bytes=size, tokens=size // 4, text_bytes=len(text.encode()))
        best = self.run.largest.get(tool)
        if best is None or size > best["bytes"]:
            self.run.largest[tool] = {
                "bytes": size,
                "text_bytes": extra["text_bytes"],
                "case": case,
            }
        refused = bool(result.get("isError"))
        if expect == "ok" and refused:
            problems.append(f"refused: {text[:300]}")
        if expect != "ok":
            if not refused:
                problems.append(f"expected a refusal, got: {text[:200]}")
            elif not text.strip() or text.lstrip().startswith(("{", "[")):
                problems.append(f"a refusal must be prose, got {text[:120]!r}")
            elif expect == "argerror":
                extra["refused_as"] = "isError"
        if contains and contains not in text:
            problems.append(f"the text lacks {contains!r}: {text[:240]!r}")
        schema = self.tools.get(tool, {}).get("outputSchema")
        sc = result.get("structuredContent")
        if refused or schema is None:
            return problems
        if sc is None:
            return problems + [
                "an outputSchema is published, no structuredContent returned"
            ]
        try:
            jsonschema.Draft202012Validator(schema).validate(sc)
        except jsonschema.ValidationError as e:
            problems.append(
                f"violates its outputSchema: {e.message} at {list(e.absolute_path)}"
            )
        if bad := self.run.deviation(tool, "structuredContent", not isinstance(sc, dict),
                                     f"structuredContent is a {type(sc).__name__}"):  # fmt: skip
            problems.append(bad)
        try:
            if len(texts) != 1 or json.loads(texts[0]) != sc:
                problems.append("the text block is not the JSON of structuredContent")
        except ValueError:
            problems.append("the text block of a structured result is not JSON")
        return problems

    def close(self) -> None:
        assert self.proc.stdin is not None
        self.proc.stdin.close()
        try:
            self.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait()
        self.reader.join(timeout=10)
        self.stderr.close()


# --------------------------------------------------------------------------- staging


def write(path: Path, text: str) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


def estate(name: str, params: dict[str, str]) -> str:
    lines = [f'  {k} = "{v}"' for k, v in params.items()]
    backend = (
        'terraform {\n  backend {\n    local { path = "terraform.tfstate" }\n  }\n}\n'
    )
    return f"estate {name}\n\nparams {{\n" + "\n".join(lines) + "\n}\n\n" + backend


def stage(work: Path) -> dict[str, Path]:
    """The smoke estate's world under one root, laid out as in the repository, so its
    `presets_dir` (../../presets) and `schema_dir` (../schemas) stay inside --root; and
    a second root whose estate has no library yet."""
    root, gp = work / "root", work / "gp"
    ignore = shutil.ignore_patterns(
        "hcl", "tmp", "evidence", "imported-*", "discovered*", "identity-*"
    )
    for rel in ("presets", "tests/smoke", "tests/schemas"):
        shutil.copytree(REPO / rel, root / rel, symlinks=True, ignore=ignore)
    for base in (root, gp):
        shutil.copytree(REPO / "presets", base / "pristine", symlinks=True)
    shutil.copytree(REPO / "tests/schemas", gp / "schemas")
    org = {"customer_organization_id": "123456789012"}
    write(gp / "yaml/gp.satz", estate("gp", org))
    write(
        gp / "config.toml",
        'yaml_dir = "yaml"\nhcl_dir = "hcl"\ninclude_dirs = [".", "yaml"]\n'
        'presets_dir = "presets"\nschema_dir = "schemas"\ntf_tool = "tofu"\n',
    )
    # the identity estates: two with their own account, and two that name none
    yaml = root / "tests/smoke/yaml"
    for short, mode, svc in (("acme", "cloud", True), ("bolt", "cloud", True),
                             ("badmode", "boot", True), ("noaccount", "cloud", False)):  # fmt: skip
        params = org | {"customer_shortname": short, "infra_project_name": f"{short}-infra-001",
                        "deployment_mode": mode}  # fmt: skip
        if svc:
            params["svc_iac_account"] = "svc-iac-001"
        write(yaml / f"identity-{short}.satz", estate(f"identity_{short}", params))
    (root / "tests/smoke/probe").mkdir()
    # offline, Checkov is a stand-in that prints one failed check, as smoke.sh's is
    report = {
        "check_type": "terraform",
        "results": {"failed_checks": [{
            "check_id": "CKV_GCP_62", "check_name": "Bucket should log access",
            "resource": "google_storage_bucket.state", "file_path": "/main.tf",
            "file_line_range": [1, 2], "guideline": None}]},
        "summary": {"passed": 3, "failed": 1, "skipped": 0, "parsing_errors": 0,
                    "resource_count": 4, "checkov_version": "3.2.0"},
    }  # fmt: skip
    fake = work / "fake-checkov"
    fixture = write(fake / "report.json", json.dumps(report))
    write(fake / "checkov", f'#!/bin/sh\ncat "{fixture}"\nexit 1\n').chmod(0o755)
    # merge-presets edits an estate only inside a repository, where the edit can be undone
    for base in (root, gp):
        for args in (
            ["init", "--quiet"],
            ["add", "--all"],
            ["commit", "--quiet", "-m", "staged"],
        ):
            vcs(base, *args)
    return {"root": root, "gp": gp, "fake_checkov": fake}


def vcs(repo: Path, *args: str) -> None:
    """git in a staged directory, with an identity of its own, no hooks, no signing."""
    settings = ("user.name=mcp-probe", "user.email=mcp-probe@example.com",
                "commit.gpgsign=false", "core.hooksPath=/dev/null")  # fmt: skip
    config = [x for s in settings for x in ("-c", s)]
    subprocess.run(["git", "-C", str(repo), *config, *args], check=True)


# --------------------------------------------------------------------------- suites


def suite_main(s: Server, root: Path, checkov: bool) -> None:
    """Every tool once, at the level it needs; merges and Checkov where they apply."""
    run = s.run
    names = set(s.tools)
    run.check(s.label, "tools/list is exactly the expected set", names == EXPECTED_TOOLS,
              f"new {sorted(names - EXPECTED_TOOLS)}, gone {sorted(EXPECTED_TOOLS - names)}")  # fmt: skip
    for name, t in sorted(s.tools.items()):
        problems = (
            []
            if "readOnlyHint" in (t.get("annotations") or {})
            else ["no readOnlyHint"]
        )
        if (schema := t.get("outputSchema")) is not None:
            try:
                jsonschema.Draft202012Validator.check_schema(schema)
            except jsonschema.SchemaError as e:
                problems.append(f"outputSchema is not JSON Schema: {e.message}")
            if bad := run.deviation(name, "outputSchema", schema.get("type") != "object",
                                    f"outputSchema type {schema.get('type')!r}"):  # fmt: skip
                problems.append(bad)
        run.check(s.label, f"tools/list: {name}", not problems, "; ".join(problems))

    s.call(
        "before satz_open, refused naming it",
        "satz_transpile_check",
        {},
        "error",
        "satz_open",
    )
    s.call("satz_estates", "satz_estates", {}, setup=True)
    s.call("satz_open smoke", "satz_open", SMOKE, setup=True)
    s.call("satz_require without framework", "satz_require", {}, "argerror")
    s.call(
        "satz_require framework mistyped", "satz_require", {"framework": 5}, "argerror"
    )
    fw, iv = (
        {"framework": FRAMEWORK, "prowler": PROWLER},
        str(root / "tests/smoke/probe/iv.satz"),
    )
    for case, tool, args, needs in (
        ("satz_fmt", "satz_fmt", {"text": 'estate e\n\nparams {\n  a  =  "1"\n}\n'}, R),
        ("satz_transpile_check", "satz_transpile_check", {}, R),
        ("satz_require", "satz_require", {"framework": FRAMEWORK}, R),
        ("satz_questions showcase", "satz_questions", {"estate": "showcase.satz"}, R),
        ("satz_interview showcase", "satz_interview", {"estate": "showcase.satz", "filter": "all"}, R),
        ("satz_triage", "satz_triage", fw, R),
        ("satz_prowler", "satz_prowler", {}, R),
        ("satz_review_pack", "satz_review_pack", {"pack": "presets/organization-budget.satz"}, R),
        ("satz_update_prerequisites report_only", "satz_update_prerequisites", {"report_only": True}, R),
        ("satz_report_compliance no_live", "satz_report_compliance", fw | {"no_live": True}, R),
        ("satz_whoami offline", "satz_whoami", {"offline": True}, R),
        ("satz_transpile", "satz_transpile", {}, W),
        ("satz_interview create", "satz_interview", {"estate": iv, "create": True}, W),
        ("satz_interview answers + accept_defaults", "satz_interview", {"estate": iv, "accept_defaults": True, "answers": {
            "customer_id": "C0example", "customer_organization_id": "123456789012",
            "customer_domain": "example.com", "customer_shortname": "acme",
            "customer_longname": "Acme", "first_admin": "first.admin",
            "billing_account_infra": "012345-6789AB-CDEF01"}}, W),
        ("satz_update_prerequisites", "satz_update_prerequisites", {}, W),
    ):  # fmt: skip
        s.call(case, tool, args, needs=needs)
    if run.mode == "offline":
        s.call("satz_adopt without credentials, refused", "satz_adopt", {}, "error")
    items = s.call(
        "satz_remediation_items",
        "satz_remediation_items",
        fw,
        setup=run.wants("satz_remediation_annotate", W),
    )
    if first := (items.get("items") or [{}])[0].get("id"):
        authored = {
            "authored_by": "mcp-probe",
            "authored_at": "2026-01-01T00:00:00Z",
            "what_why": "probe",
        }
        s.call("satz_remediation_annotate", "satz_remediation_annotate",
               fw | {"out": "tests/smoke/probe/plan", "dossier_sha256": items["dossier_sha256"],
                     "items": {first: authored}}, needs=W)  # fmt: skip
    elif run.wants("satz_remediation_annotate", W):
        run.record(
            s.label,
            "satz_remediation_annotate",
            "FAIL",
            "no remediation item to annotate",
        )
    if run.wants("satz_scan_checkov", X, "offline"):
        if not checkov:
            run.record(
                s.label,
                "satz_scan_checkov",
                "SKIP",
                "neither checkov nor uvx is on PATH",
            )
        else:
            s.call("satz_transpile, for the scan", "satz_transpile", {}, setup=True)
            out = "tests/smoke/probe/checkov.json"
            s.call(
                "satz_scan_checkov",
                "satz_scan_checkov",
                {"out": out},
                needs=X,
                timeout=900,
            )
            s.call(
                "satz_remediation_items + checkov",
                "satz_remediation_items",
                fw | {"checkov": out},
                needs=X,
            )
    s.call("satz_check_presets", "satz_check_presets", {}, mode="net", timeout=600)


def suite_gp(s: Server) -> None:
    """get-presets fills an estate's library from a pristine one; merge-presets reports,
    then writes."""
    s.call(
        "satz_open gp",
        "satz_open",
        {"config": "config.toml", "estate": "gp.satz"},
        setup=True,
    )
    pristine = {"pristine_dir": "pristine"}
    s.call("satz_get_presets pristine_dir", "satz_get_presets", pristine, needs=W)
    s.call(
        "satz_merge_presets report_only",
        "satz_merge_presets",
        pristine | {"report_only": True},
        needs=W,
    )
    s.call("satz_merge_presets (writes)", "satz_merge_presets", pristine, needs=W)


def suite_ceiling(s: Server, outside: Path) -> None:
    """--allow read: every write and exec tool refused naming its group; a path outside
    the root refused alike whether or not something is there."""
    s.call("satz_open smoke", "satz_open", SMOKE, setup=True)
    fw = {"framework": FRAMEWORK, "prowler": PROWLER}
    for tool, args, group in (
        ("satz_transpile", {}, "write"),
        ("satz_update_prerequisites", {}, "write"),
        ("satz_interview", {"answers": {"customer_id": "C0example"}}, "write"),
        ("satz_adopt", {"execute": True}, "write"),
        ("satz_get_presets", {"pristine_dir": "pristine"}, "write"),
        (
            "satz_merge_presets",
            {"pristine_dir": "pristine", "report_only": True},
            "write",
        ),
        (
            "satz_remediation_annotate",
            fw | {"out": "x", "dossier_sha256": "0" * 64, "items": {}},
            "write",
        ),
        ("satz_scan_checkov", {}, "exec"),
    ):
        s.call(f"--allow read refuses {tool}", tool, args, "error", f"needs '{group}'")
    s.call(
        "satz_restrict without --self-gated, refused",
        "satz_restrict",
        {"allow": "read"},
        "error",
    )
    missing = outside.parent / "mcp-probe-no-such.satz"
    s.call(
        "satz_open config outside the root",
        "satz_open",
        {"config": str(outside.parent), "estate": "x.satz"},
        "error",
        OUTSIDE,
    )
    s.call(
        "satz_triage prowler outside the root",
        "satz_triage",
        fw | {"prowler": str(outside)},
        "error",
        OUTSIDE,
    )
    s.call(
        "satz_review_pack pack outside the root",
        "satz_review_pack",
        {"pack": str(outside)},
        "error",
        OUTSIDE,
    )
    if s.run.wants("satz_require", R):
        said = []
        for path in (outside, missing):
            r = s.request(
                "tools/call",
                {
                    "name": "satz_require",
                    "arguments": {"framework": FRAMEWORK, "estate": str(path)},
                },
            )
            text = "".join(
                c.get("text", "") for c in r.get("result", {}).get("content", [])
            )
            said.append(text.replace(str(path), "<path>"))
        s.run.check(s.label, "outside the root, an existing and a missing path read the same",
                    said[0] == said[1] and OUTSIDE in said[0], repr(said))  # fmt: skip


def suite_gated(s: Server) -> None:
    """--self-gated: satz_restrict narrows, and never widens again."""
    s.call("satz_open smoke", "satz_open", SMOKE, setup=True)
    s.call("satz_restrict to read", "satz_restrict", {"allow": "read"})
    s.call(
        "a write after the restrict, refused",
        "satz_transpile",
        {},
        "error",
        "needs 'write'",
        needs=W,
    )
    s.call(
        "satz_restrict cannot widen", "satz_restrict", {"allow": "read,write"}, "error"
    )


def suite_identity(s: Server) -> None:
    """runs_as and whoami follow the open estate; an estate that names no identity is
    refused by every call that would act as it."""
    run = s.run
    for short in ("acme", "bolt", "acme"):
        want = f"svc-iac-001@{short}-infra-001.iam.gserviceaccount.com"
        opened = s.call(f"satz_open identity-{short}", "satz_open",
                        {"config": SMOKE["config"], "estate": f"identity-{short}.satz"}, setup=True)  # fmt: skip
        run.check(
            s.label,
            f"runs_as follows identity-{short}",
            opened.get("runs_as") == want,
            str(opened),
        )
        who = s.call(
            f"satz_whoami after identity-{short}", "satz_whoami", {"offline": True}
        )
        if run.wants("satz_whoami", R):
            got = who.get("estate", {}).get("service_account")
            run.check(
                s.label, f"whoami answers for identity-{short}", got == want, repr(got)
            )
    for tool, args in (
        ("satz_open", {"config": SMOKE["config"], "estate": "identity-badmode.satz"}),
        ("satz_whoami", {"offline": True, "estate": "identity-noaccount.satz"}),
        ("satz_adopt", {"estate": "identity-badmode.satz"}),
        (
            "satz_report_compliance",
            {
                "framework": FRAMEWORK,
                "no_live": True,
                "estate": "identity-noaccount.satz",
            },
        ),
    ):
        s.call(
            f"{tool}: an estate that names no identity, refused",
            tool,
            args,
            "error",
            NO_IDENTITY,
        )


def suite_live(s: Server, config: str, main_satz: str) -> None:
    """ADC on the test organisation. Reads only: adopt runs without execute."""
    opened = s.call(
        "satz_open the live estate",
        "satz_open",
        {"config": config, "estate": main_satz},
        setup=True,
    )
    who = s.call("satz_whoami online", "satz_whoami", {}, mode="live", timeout=300)
    if s.run.wants("satz_whoami", R, "live"):
        # a cloud-mode estate runs as its account; a local-mode one (runs_as null) as
        # the ADC itself, impersonating nothing
        est, runs_as = who.get("estate", {}), opened.get("runs_as")
        ok = (
            est.get("service_account") == runs_as
            if runs_as
            else est.get("impersonated") is False
        )
        s.run.check(
            s.label,
            "whoami online answers what satz_open said",
            ok,
            f"{est} vs {runs_as!r}",
        )
    rep = s.call(
        "satz_report_compliance live",
        "satz_report_compliance",
        {"framework": FRAMEWORK},
        mode="live",
        timeout=900,
    )
    if s.run.wants("satz_report_compliance", R, "live"):
        s.run.check(
            s.label,
            "the inventory was read",
            rep.get("live_status") == "verified",
            str(rep.get("warnings")),
        )
    s.call("satz_adopt, resolve only", "satz_adopt", {}, mode="live", timeout=900)


# --------------------------------------------------------------------------- main


def report(
    run: Run, servers: list[Server], work: Path, version: str, elapsed: float
) -> bool:
    strays = [(srv.label, where, line) for srv in servers for where, line in srv.stray]
    print("\n== stdout lines that are not JSON-RPC ==")
    for label, where, line in strays:
        print(f"  [{label}] during {where}: {line}")
    if not strays:
        print("  none")
    print(f"\n== MCP {PROTOCOL} deviations ==")
    for key, why in sorted(KNOWN_DEVIATIONS.items()):
        if key in run.deviations:
            print(f"  KNOWN  {key[0]} {key[1]}: {why}")
        elif not run.tools or key[0] in run.tools:
            run.record(
                "-",
                f"{key[0]} {key[1]}: listed, no longer occurs",
                "FAIL",
                "remove it from KNOWN_DEVIATIONS",
            )
    for key in sorted(run.deviations - set(KNOWN_DEVIATIONS)):
        print(f"  NEW    {key[0]} {key[1]}")
    print("\n== how a missing or mistyped argument is refused ==")
    for r in run.results:
        if "refused_as" in r:
            print(f"  {r['refused_as']:15} {r['case']}")
    print("\n== largest result per tool (tokens estimated at 4 bytes each) ==")
    print(f"  {'tool':28} {'bytes':>9} {'~tokens':>8} {'text':>9}  case")
    for tool, big in sorted(run.largest.items(), key=lambda kv: -kv[1]["bytes"]):
        print(
            f"  {tool:28} {big['bytes']:9d} {big['bytes'] // 4:8d} {big['text_bytes']:9d}  {big['case']}"
        )
    fails = sum(r["status"] == "FAIL" for r in run.results)
    skips = sum(r["status"] == "SKIP" for r in run.results)
    print(f"\n{len(run.results)} checks: {fails} FAIL, {skips} SKIP, {len(strays)} stray line(s), "
          f"{elapsed:.1f}s; logs in {work}")  # fmt: skip
    data = {"satz": version, "mode": run.mode, "groups": sorted(run.groups), "seconds": round(elapsed, 1),
            "results": run.results, "largest": run.largest, "stray": strays,
            "deviations": sorted(" ".join(d) for d in run.deviations)}  # fmt: skip
    (work / "report.json").write_text(json.dumps(data, indent=2))
    return not fails and not strays


def main() -> None:
    ap = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    ap.add_argument(
        "--satz",
        default=str(REPO / "target/release/satz"),
        help="the binary (default: target/release/satz)",
    )
    ap.add_argument(
        "--work",
        help="the directory to stage into and log to (default: a new temporary one)",
    )
    ap.add_argument("--mode", choices=MODES, default="offline")
    ap.add_argument(
        "--groups",
        default="read,write,exec",
        help="capability groups: read[,write[,exec]]",
    )
    ap.add_argument(
        "--tools", default="", help="only these tools' cases, comma-separated"
    )
    ap.add_argument("--live-dir", help="live: the estate checkout to copy and open")
    ap.add_argument(
        "--live-config",
        default="config.toml",
        help="live: its config, relative to --live-dir",
    )
    ap.add_argument("--live-estate", help="live: its main .satz")
    a = ap.parse_args()

    satz = Path(a.satz).resolve()
    if not os.access(satz, os.X_OK):
        sys.exit(
            f"mcp-probe: {satz} is not an executable: `cargo build --release`, or pass --satz"
        )
    groups = {g.strip() for g in a.groups.split(",") if g.strip()}
    if "read" not in groups or not groups <= set(GROUPS):
        sys.exit(
            f"mcp-probe: --groups is read, optionally with write and exec, not {a.groups!r}"
        )
    tools = {t.strip() for t in a.tools.split(",") if t.strip()}
    if unknown := tools - EXPECTED_TOOLS:
        sys.exit(f"mcp-probe: no such tool: {sorted(unknown)}")
    if a.mode == "live" and not (a.live_dir and a.live_estate):
        sys.exit("mcp-probe: --mode live needs --live-dir and --live-estate")
    work = (
        Path(a.work).resolve()
        if a.work
        else Path(tempfile.mkdtemp(prefix="mcp-probe-"))
    )
    shutil.rmtree(work, ignore_errors=True)
    work.mkdir(parents=True)

    run, t0 = Run(groups, a.mode, tools), time.monotonic()
    version = subprocess.run(
        [satz, "--version"], capture_output=True, text=True, check=True
    ).stdout.strip()
    allow = ",".join(g for g in GROUPS if g in groups)
    print(f"mcp-probe: {version}, mode {a.mode}, groups {allow}, work {work}")
    paths = stage(work)
    # offline, nothing can mint a token: the ADC path names a file that does not exist
    env = os.environ | {"GOOGLE_APPLICATION_CREDENTIALS": str(work / "no-adc.json")}
    main_env, checkov = env, True
    if a.mode == "offline":
        main_env = env | {
            "PATH": f"{paths['fake_checkov']}{os.pathsep}{env.get('PATH', '')}"
        }
    else:
        checkov = bool(shutil.which("checkov") or shutil.which("uvx"))

    servers: list[Server] = []

    def start(label: str, root: Path, allow_: str, env_: dict, *flags: str) -> Server:
        servers.append(Server(run, label, satz, root, allow_, work, env_, *flags))
        return servers[-1]

    try:
        suite_main(
            start("main", paths["root"], allow, main_env), paths["root"], checkov
        )
        if "write" in groups and (
            not tools or tools & {"satz_get_presets", "satz_merge_presets"}
        ):
            suite_gp(start("gp", paths["gp"], "read,write", env))
        suite_ceiling(start("ceiling", paths["root"], "read", env), REPO / "README.md")
        suite_gated(start("gated", paths["root"], allow, env, "--self-gated"))
        suite_identity(start("identity", paths["root"], "read", env))
        if a.mode == "live":
            live = work / "live"
            ignore = shutil.ignore_patterns(".git", ".terraform", "evidence")
            shutil.copytree(
                Path(a.live_dir).expanduser(), live, symlinks=True, ignore=ignore
            )
            suite_live(
                start("live", live, "read", dict(os.environ)),
                a.live_config,
                a.live_estate,
            )
    finally:
        for srv in servers:
            srv.close()
    sys.exit(0 if report(run, servers, work, version, time.monotonic() - t0) else 1)


if __name__ == "__main__":
    main()
