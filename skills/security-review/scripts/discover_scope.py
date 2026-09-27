#!/usr/bin/env python3
"""Discover a customer's review scope from its folder under ~/projects/ccc/<shortcode>/.

    python3 discover_scope.py <shortcode-or-folder> [--scan-date YYYY-MM-DD] [--write <scope.yaml>]

Reads the central estate repo (<shortcode>-C<dirid>/: config.toml, whose `yaml_dir` names the
estate directory — `satz/` when the key is absent — and <id>.satz inside it) and prints the facts
every later step needs: org id, domain, infra project, IaC service account, estate file, audit
folder, the workloads beside the repo and the own project ids for Prowler. With --write it seeds
a scope.yaml from assets/scope.template.yaml. `satz interfaces` runs when satz is on PATH; it
compiles the estate offline. Nothing here touches the cloud.

A workload is a sibling directory of the central repo inside the customer folder, neither the
repo nor `audit/`, of one of two kinds: `satz` — a project estate (config.toml, a .satz in its
yaml_dir with a `use "…/interface.satz"` line) — or `hcl` — a directory of .tf files reading
`module.satz.<export>`. Anything else (documents, pdfs) is ignored.
"""

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

CCC = Path(os.environ.get("CCC_ROOT", Path.home() / "projects" / "ccc"))
HERE = Path(__file__).resolve().parent
USE_INTERFACE = re.compile(r'^\s*use\s+"([^"]*interface\.satz)"', re.M)
MODULE_SATZ = re.compile(r"\bmodule\.satz\.[A-Za-z_]")
MODULE_SOURCE = re.compile(r'source\s*=\s*"[^"]*?/([^/"]+)/hcl"')
KIND_SATZ, KIND_HCL = "satz", "hcl"


def find_customer(arg):
    p = Path(arg).expanduser()
    if p.is_dir() and (
        p.parent == CCC
        or any(c.name.startswith(p.name + "-C") for c in p.iterdir() if c.is_dir())
    ):
        return p
    q = CCC / arg
    if q.is_dir():
        return q
    sys.exit(
        f"no customer folder for {arg!r} under {CCC} (expected {CCC}/<shortcode>/<shortcode>-C<dirid>/)"
    )


def find_repo(cust):
    repos = sorted(
        c
        for c in cust.iterdir()
        if c.is_dir() and re.match(rf"^{re.escape(cust.name)}-C[0-9a-z]+$", c.name)
    )
    if len(repos) != 1:
        sys.exit(
            f"expected exactly one estate repo {cust.name}-C<dirid> in {cust}, found {[r.name for r in repos]}"
        )
    return repos[0]


def config_key(cfg_text, key, default):
    """A string key of config.toml; `default` when the file does not set it."""
    m = re.search(rf'^\s*{key}\s*=\s*"([^"]+)"', cfg_text, re.M)
    return m.group(1) if m else default


def parse_params(satz_text):
    m = re.search(r"params\s*\{(.*?)\n\}", satz_text, re.S)
    out = {}
    if not m:
        return out
    for line in m.group(1).splitlines():
        line = line.split("//")[0].strip()
        mm = re.match(r'^([a-z_0-9]+)\s*=\s*"([^"]*)"', line)
        if mm:
            out[mm.group(1)] = mm.group(2)
    return out


def interface_of_use(path):
    """`vendor/<project>/<interface>/satz/interface.satz` → `<interface>`; "" when the path has another shape."""
    parts = path.split("/")
    return parts[-3] if len(parts) >= 3 and parts[-2] == "satz" else ""


def find_workloads(cust, repo):
    """The workloads beside the central repo: [{name, kind, folder, interface, hcl_dir}]."""
    out = []
    for d in sorted(cust.iterdir()):
        if not d.is_dir() or d == repo or d.name == "audit" or d.name.startswith("."):
            continue
        cfg = d / "config.toml"
        if cfg.exists():
            cfg_text = cfg.read_text()
            ydir = d / config_key(cfg_text, "yaml_dir", "satz")
            for f in sorted(ydir.glob("*.satz")) if ydir.is_dir() else []:
                m = USE_INTERFACE.search(f.read_text(errors="replace"))
                if m:
                    out.append(
                        dict(
                            name=f.stem,
                            kind=KIND_SATZ,
                            folder=d.name,
                            interface=interface_of_use(m.group(1)),
                            hcl_dir=config_key(cfg_text, "hcl_dir", "hcl"),
                        )
                    )
                    break
            continue
        tfs = [
            t
            for t in list(d.glob("*.tf")) + list(d.glob("*/*.tf"))
            if ".terraform" not in t.parts
        ]
        if not tfs:
            continue
        text = "\n".join(t.read_text(errors="replace") for t in tfs)
        if MODULE_SATZ.search(text):
            m = MODULE_SOURCE.search(text)
            out.append(
                dict(
                    name=d.name,
                    kind=KIND_HCL,
                    folder=d.name,
                    interface=m.group(1) if m else "",
                    hcl_dir="",
                )
            )
    return out


def read_interfaces(repo, estate_file):
    """`satz interfaces --format json`: the report, or (None, reason) when it cannot be read."""
    if not shutil.which("satz"):
        return None, "satz is not on PATH"
    r = subprocess.run(
        [
            "satz",
            "--config",
            str(repo),
            "interfaces",
            estate_file.name,
            "--format",
            "json",
            "--out",
            "-",
        ],
        capture_output=True,
        text=True,
    )
    if r.returncode != 0:
        tail = [x for x in r.stderr.strip().splitlines() if x.strip()][-1:]
        return None, f"satz interfaces failed ({r.returncode}): {' '.join(tail)}"
    try:
        return json.loads(r.stdout), ""
    except json.JSONDecodeError as e:
        return None, f"satz interfaces wrote no JSON: {e}"


def unquote(v):
    return v[1:-1] if len(v) >= 2 and v[0] == v[-1] == '"' else v


def project_ids_of(report):
    """{interface: google project id} from the static `project_id` export of each interface."""
    return {
        e["interface"]: unquote(e["value"])
        for e in report.get("exports", [])
        if e.get("interface") and e["name"] == "project_id" and e["how"] == "static"
    }


def declared_projects(repo):
    """The `google_project` ids the estate's generated root module declares (hcl/main.tf)."""
    main_tf = repo / "hcl" / "main.tf"
    if not main_tf.exists():
        return []
    ids = []
    for m in re.finditer(
        r'^resource\s+"google_project"\s+"[^"]+"\s*\{(.*?)^\}',
        main_tf.read_text(errors="replace"),
        re.M | re.S,
    ):
        pid = re.search(r'^\s*project_id\s*=\s*"([^"]+)"', m.group(1), re.M)
        if pid:
            ids.append(pid.group(1))
    return ids


def yaml_workloads(workloads):
    """The `workloads:` list for scope.yaml, without a YAML library (plain python3 runs this)."""
    if not workloads:
        return "workloads: []"
    lines = ["workloads:"]
    for w in workloads:
        lines.append(f"  - name: {json.dumps(w['name'])}")
        for k in ("kind", "folder", "interface", "project_id"):
            lines.append(f"    {k}: {json.dumps(w.get(k, ''))}")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("customer")
    ap.add_argument("--scan-date", default=datetime.date.today().isoformat())
    ap.add_argument("--write", help="write a seeded scope.yaml here")
    a = ap.parse_args()
    cust = find_customer(a.customer)
    repo = find_repo(cust)
    cfg = (repo / "config.toml").read_text() if (repo / "config.toml").exists() else ""
    yaml_dir = repo / config_key(cfg, "yaml_dir", "satz")
    estate_id = repo.name.split("-", 1)[1]
    estate_file = yaml_dir / f"{estate_id}.satz"
    if not estate_file.exists():
        cands = list(yaml_dir.glob("*.satz"))
        if len(cands) == 1:
            estate_file = cands[0]
        else:
            sys.exit(f"estate file not found: {estate_file}")
    p = parse_params(estate_file.read_text())
    org = p.get("customer_organization_id", "")
    dom = p.get("customer_domain", "")
    infra = p.get("infra_project_name", "")
    sa = p.get("svc_iac_account", "svc-iac-001")
    workloads = find_workloads(cust, repo)
    report, why = read_interfaces(repo, estate_file)
    iface_projects = project_ids_of(report) if report else {}
    for w in workloads:
        w["project_id"] = iface_projects.get(w["interface"], "")
    own = [infra] if infra else []
    for pid in declared_projects(repo) + sorted(iface_projects.values()):
        if pid and pid not in own:
            own.append(pid)
    facts = {
        "customer_folder": str(cust),
        "estate_repo": str(repo),
        "estate_dir": yaml_dir.name,
        "estate_file": estate_file.name,
        "shortcode": p.get("customer_shortname", cust.name),
        "longname": p.get("customer_longname", ""),
        "org_id": org,
        "domain": dom,
        "infra_project": infra,
        "iac_sa": f"{sa}@{infra}.iam.gserviceaccount.com" if infra else "",
        "audit_dir": str(cust / "audit"),
        "scan_dir": str(cust / "audit" / dom / a.scan_date),
        "scan_date": a.scan_date,
        "own_projects": " ".join(own),
    }
    for k, v in facts.items():
        print(f"{k}={v}")
    for w in workloads:
        print(f"workload={w['name']}:{w['kind']}:{w['folder']}")
    print(
        "workloads_arg="
        + " ".join(f"{w['name']}:{w['kind']}:{w['folder']}" for w in workloads)
    )
    missing = [k for k in ("org_id", "domain", "infra_project") if not facts[k]]
    if missing:
        print(
            f"WARNING: could not read {missing} from {estate_file} — fill scope.yaml by hand",
            file=sys.stderr,
        )
    if report is None:
        print(
            f"WARNING: {why} — own_projects holds only the infra project and hcl/main.tf's google_project ids; "
            "fill the workloads' project_id in scope.yaml and the --own-projects list by hand",
            file=sys.stderr,
        )
    for w in workloads:
        if not w["project_id"]:
            print(
                f"WARNING: workload {w['name']} ({w['kind']}): no static project_id export on interface "
                f"{w['interface'] or '?'} — fill workloads[].project_id in scope.yaml by hand",
                file=sys.stderr,
            )
    if a.write:
        tpl = (HERE.parent / "assets" / "scope.template.yaml").read_text()
        d = datetime.date.fromisoformat(a.scan_date)
        rep = {
            "SHORTCODE": facts["shortcode"],
            "LONGNAME": facts["longname"] or facts["shortcode"],
            "DOMAIN": dom,
            "ORG_ID": org,
            "SCAN_DATE": a.scan_date,
            "SCAN_DATE_DE": d.strftime("%d.%m.%Y"),
            "INFRA_PROJECT": infra,
            "ESTATE_REPO": facts["estate_repo"],
            "ESTATE_FILE": facts["estate_file"],
            "IAC_SA": facts["iac_sa"],
            "WORKLOADS": yaml_workloads(workloads),
        }
        for k, v in rep.items():
            tpl = tpl.replace("{{" + k + "}}", v)
        out = Path(a.write).expanduser()
        out.parent.mkdir(parents=True, exist_ok=True)
        if out.exists():
            print(f"NOTE: {out} exists — not overwritten", file=sys.stderr)
        else:
            out.write_text(tpl)
            print(f"wrote {out}")


if __name__ == "__main__":
    main()
