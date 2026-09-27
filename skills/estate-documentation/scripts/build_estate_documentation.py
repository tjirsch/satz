#!/usr/bin/env python3
"""Cloud-Cockpit-Kurzdokumentation aus einem satz-Estate.

    uv run --with python-docx --with matplotlib python3 build_estate_documentation.py \
        --shortcode acme [--index 01] [--out-dir ~/projects/ccc/acme]

Erzeugt <NN>-<sc>-<estate>-answers.xlsx (satz questions) und
<NN>-<sc>-Konfiguration-Kurzbeschreibung.docx samt Diagramm. Alles aus .satz, hcl/main.tf und
`satz interfaces` (die Projekte neben dem Estate: Interfaces, Request-Punkte, Workload-Ordner).

Kundenordner: ~/projects/ccc/<sc>/ (CCC_ROOT ueberschreibt ~/projects/ccc). Darin das zentrale
Estate-Repo <sc>-C<dirid>/ — config.toml, dessen `yaml_dir` das Estate-Verzeichnis nennt (ohne den
Schluessel `satz/`) — und daneben null oder mehr Workload-Ordner: ein satz-Projekt (config.toml,
eine .satz mit `use "…/interface.satz"`) oder ein HCL-Projekt (.tf-Dateien, die
`module.satz.<export>` lesen). Alles andere daneben (Dokumente, PDFs) ist kein Workload.
"""

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

CCC = Path(os.environ.get("CCC_ROOT", Path.home() / "projects" / "ccc"))
USE_INTERFACE = re.compile(r'^\s*use\s+"([^"]*interface\.satz)"', re.M)
MODULE_SATZ = re.compile(r"\bmodule\.satz\.[A-Za-z_]")
MODULE_SOURCE = re.compile(r'source\s*=\s*"[^"]*?/([^/"]+)/hcl"')
KIND_DE = {
    "satz": "satz-Projekt (eigenes Estate)",
    "hcl": "HCL-Projekt (Terraform)",
}


# ---------- Quellen lesen -------------------------------------------------
def config_key(cfg_text, key, default):
    """Ein String-Schluessel der config.toml; `default`, wenn die Datei ihn nicht setzt."""
    m = re.search(rf'^\s*{key}\s*=\s*"([^"]+)"', cfg_text, re.M)
    return m.group(1) if m else default


def find_repo(shortcode, repo=None):
    if repo:
        return Path(repo).expanduser()
    cust = CCC / shortcode
    hits = [
        p
        for p in sorted(cust.glob(f"{shortcode}-C*"))
        if p.is_dir()
        and (p / "config.toml").exists()
        and re.match(rf"^{re.escape(shortcode)}-C[0-9a-z]+$", p.name)
    ]
    if len(hits) != 1:
        sys.exit(
            f"erwartet genau ein Estate-Repo {shortcode}-C<dirid> unter {cust}, gefunden: {[h.name for h in hits]}"
        )
    return hits[0]


def find_estate(repo):
    cfg = repo / "config.toml"
    ydir = repo / config_key(
        cfg.read_text() if cfg.exists() else "", "yaml_dir", "satz"
    )
    hits = sorted(ydir.glob("*.satz"))
    if not hits:
        sys.exit(f"keine .satz in {ydir} (yaml_dir aus config.toml, Vorgabe satz/)")
    return hits[0]


def interface_of_use(path):
    """`vendor/<project>/<interface>/satz/interface.satz` → `<interface>`; "" bei anderer Form."""
    parts = path.split("/")
    return parts[-3] if len(parts) >= 3 and parts[-2] == "satz" else ""


def find_workloads(cust, repo):
    """Die Workloads neben dem Estate-Repo: [{name, kind, folder, interface}]."""
    out = []
    for d in sorted(cust.iterdir()):
        if not d.is_dir() or d == repo or d.name == "audit" or d.name.startswith("."):
            continue
        cfg = d / "config.toml"
        if cfg.exists():
            ydir = d / config_key(cfg.read_text(), "yaml_dir", "satz")
            for f in sorted(ydir.glob("*.satz")) if ydir.is_dir() else []:
                m = USE_INTERFACE.search(f.read_text(errors="replace"))
                if m:
                    out.append(
                        dict(
                            name=f.stem,
                            kind="satz",
                            folder=d.name,
                            interface=interface_of_use(m.group(1)),
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
                    kind="hcl",
                    folder=d.name,
                    interface=m.group(1) if m else "",
                )
            )
    return out


def read_interfaces(repo, satz_path):
    """`satz interfaces --format json`: was das Estate den Projekten veroeffentlicht. Bricht ab,
    wenn satz fehlt oder das Estate nicht kompiliert — ein Dokument ohne diesen Abschnitt waere
    unvollstaendig, ohne dass man es ihm ansieht."""
    if not shutil.which("satz"):
        sys.exit(
            "satz ist nicht im PATH — `satz interfaces` liefert den Abschnitt Workloads"
        )
    cmd = [
        "satz",
        "--config",
        str(repo),
        "interfaces",
        satz_path.name,
        "--format",
        "json",
        "--out",
        "-",
    ]
    print("$", " ".join(cmd))
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        print(r.stderr[-1500:], file=sys.stderr)
        sys.exit(f"satz interfaces schlug fehl ({r.returncode})")
    return json.loads(r.stdout)


def unquote(v):
    return v[1:-1] if len(v) >= 2 and v[0] == v[-1] == '"' else v


def interfaces_view(report):
    """Der Report als Sicht fuer Dokument und Diagramm: Projekt-Interfaces mit Google-Projekt und
    dem, was sie lesen; die gemeinsamen Interfaces; die Request-Punkte; der Workload-Ordner."""
    exports = report.get("exports", [])
    commons = [i["name"] for i in report.get("interfaces", []) if i.get("common")]
    pid_of = {
        e["interface"]: unquote(e["value"])
        for e in exports
        if e.get("interface") and e["name"] == "project_id" and e["how"] == "static"
    }
    projects = []
    for i in report.get("interfaces", []):
        if i.get("common"):
            continue
        reads = ["core"] + [f"{c} (common)" for c in commons]
        reads += [u for u in i.get("uses", []) if u not in commons]
        projects.append(
            dict(
                name=i["name"],
                project_id=pid_of.get(i["name"], ""),
                reads=reads,
                exports=i.get("exports", 0),
            )
        )
    requests = [
        dict(
            param=f"contributes_{q['param']}",
            key=q["key"],
            fields=q.get("fields", []),
            entries=q.get("entries", 0),
            description=q.get("description") or "",
        )
        for q in report.get("requests", [])
    ]
    wf = next(
        (
            e
            for e in exports
            if e["name"] == "workload_folder" and not e.get("interface")
        ),
        None,
    )
    return dict(
        projects=projects,
        commons=commons,
        requests=requests,
        workload_folder=bool(
            wf and any(t.startswith("google_folder.") for t in wf.get("targets", []))
        ),
    )


def read_params(satz_path):
    """params { key = value } — Strings, Bools, Listen als Rohtext."""
    text = satz_path.read_text(encoding="utf-8")
    m = re.search(r"\bparams\s*\{", text)
    if not m:
        return {}
    i, depth = m.end(), 1
    while i < len(text) and depth:
        if text[i] == "{":
            depth += 1
        elif text[i] == "}":
            depth -= 1
        i += 1
    body = text[m.end() : i - 1]
    out = {}
    for line in body.splitlines():
        line = line.split("//")[0].strip()
        mm = re.match(r"^([a-z0-9_]+)\s*=\s*(.+?)\s*$", line)
        if mm:
            out[mm.group(1)] = mm.group(2).strip().strip('"')
    return out


def read_hcl(repo):
    cfg = repo / "config.toml"
    hcl_dir = config_key(cfg.read_text() if cfg.exists() else "", "hcl_dir", "hcl")
    p = repo / hcl_dir / "main.tf"
    return p.read_text(encoding="utf-8") if p.exists() else ""


def blocks(hcl, tf_type):
    """[(name, body)] je resource-Block eines Typs."""
    out = []
    for m in re.finditer(
        r'^resource\s+"%s"\s+"([a-z0-9_]+)"\s*\{' % re.escape(tf_type), hcl, re.M
    ):
        i, depth = m.end(), 1
        while i < len(hcl) and depth:
            if hcl[i] == "{":
                depth += 1
            elif hcl[i] == "}":
                depth -= 1
            i += 1
        out.append((m.group(1), hcl[m.end() : i - 1]))
    return out


def attr(body, key):
    m = re.search(r'^\s*%s\s*=\s*"([^"]*)"' % re.escape(key), body, re.M)
    return m.group(1) if m else None


def count(hcl, tf_type):
    return len(re.findall(r'^resource\s+"%s"\s' % re.escape(tf_type), hcl, re.M))


def collect(repo, satz_path, report, workloads):
    p, hcl = read_params(satz_path), read_hcl(repo)
    folders = [
        (attr(b, "display_name") or n, attr(b, "parent") or "")
        for n, b in blocks(hcl, "google_folder")
    ]
    projects = []
    svc_blocks = blocks(hcl, "google_project_service")
    for n, b in blocks(hcl, "google_project"):
        pid = attr(b, "project_id") or ""
        svcs = []
        for _sn, sb in svc_blocks:
            # `project` is usually an unquoted reference (google_project.<key>.project_id),
            # occasionally the literal id — accept both.
            ref = re.search(r"^\s*project\s*=\s*(.+?)\s*$", sb, re.M)
            target = ref.group(1).strip().strip('"') if ref else ""
            if target == f"google_project.{n}.project_id" or (pid and target == pid):
                sv = attr(sb, "service")
                if sv:
                    svcs.append(sv)
        projects.append(
            {"key": n, "id": pid or n, "name": attr(b, "name"), "services": svcs}
        )
    groups = sorted(
        {
            attr(b, "id") or attr(b, "group_key") or n
            for n, b in blocks(hcl, "google_cloud_identity_group")
        }
        - {None}
    )
    if not groups:
        groups = sorted(
            set(
                re.findall(
                    r'"([a-z0-9._-]+@%s)"' % re.escape(p.get("customer_domain", "")),
                    hcl,
                )
            )
        )
    buckets = [(attr(b, "name") or n) for n, b in blocks(hcl, "google_storage_bucket")]
    sinks = [
        (n, attr(b, "name") or n)
        for n, b in blocks(hcl, "google_logging_organization_sink")
    ]
    topics = [(attr(b, "name") or n) for n, b in blocks(hcl, "google_pubsub_topic")]
    all_services = sorted(
        {attr(b, "service") for _, b in blocks(hcl, "google_project_service")} - {None}
    )
    # Sicherheitsmodell: security_model_<id> = true
    model = next(
        (
            k.split("security_model_", 1)[1].upper()
            for k, v in sorted(p.items())
            if k.startswith("security_model_") and v == "true"
        ),
        None,
    )

    # Rollen je Gruppe aus dem erzeugten HCL — was tatsaechlich gebunden ist.
    roles = {}
    for tf in ("google_organization_iam_member", "google_billing_account_iam_member"):
        for _n, b in blocks(hcl, tf):
            mem, role = attr(b, "member"), attr(b, "role")
            if mem and role and mem.startswith("group:"):
                roles.setdefault(mem[len("group:") :], []).append(role)

    group_details = []
    for n, b in blocks(hcl, "google_cloud_identity_group"):
        km = re.search(r"group_key\s*\{[^}]*?id\s*=\s*\"([^\"]+)\"", b, re.S)
        email = km.group(1) if km else (attr(b, "id") or n)
        desc = (attr(b, "description") or "").replace("\\n", " ").strip()
        desc = re.sub(r"\s+", " ", desc)
        group_details.append(
            {
                "email": email,
                "display_name": attr(b, "display_name") or email.split("@")[0],
                "description": desc,
                "roles": sorted(set(roles.get(email, []))),
            }
        )
    group_details.sort(key=lambda g: g["email"])

    packs = sorted(k for k, v in p.items() if k.startswith("use_") and v == "true")
    packs_off = sorted(k for k, v in p.items() if k.startswith("use_") and v == "false")
    iv = interfaces_view(report)
    return {
        "params": p,
        "folders": folders,
        "projects": projects,
        "groups": groups,
        "buckets": buckets,
        "sinks": sinks,
        "topics": topics,
        "services": all_services,
        "security_model": model,
        "group_details": group_details,
        "packs_on": packs,
        "packs_off": packs_off,
        "interfaces": iv,
        "workloads": workloads,
        "n": {
            t: count(hcl, t)
            for t in [
                "google_org_policy_policy",
                "google_org_policy_custom_constraint",
                "google_organization_iam_member",
                "google_monitoring_alert_policy",
                "google_logging_metric",
                "google_cloud_identity_group_membership",
                "google_compute_firewall_policy_rule",
                "google_billing_account_iam_member",
                "google_project_service",
                "google_essential_contacts_contact",
            ]
        },
    }


# ---------- Diagramm ------------------------------------------------------
def draw(facts, png):
    import matplotlib

    matplotlib.use("Agg")
    import matplotlib.pyplot as plt
    from matplotlib.patches import FancyBboxPatch

    p = facts["params"]
    iv = facts["interfaces"]
    wl_projects = iv["projects"]
    wide = bool(wl_projects) or iv["workload_folder"]
    fig, ax = plt.subplots(figsize=(13.4 if wide else 10.5, 6.4))
    ax.set_xlim(0, 132 if wide else 100)
    ax.set_ylim(0, 100)
    ax.axis("off")

    def box(x, y, w, h, title, lines, fc, ec):
        ax.add_patch(
            FancyBboxPatch(
                (x, y),
                w,
                h,
                boxstyle="round,pad=0.6,rounding_size=1.4",
                fc=fc,
                ec=ec,
                lw=1.4,
            )
        )
        ax.text(
            x + w / 2,
            y + h - 3.4,
            title,
            ha="center",
            va="top",
            fontsize=9.5,
            fontweight="bold",
        )
        for i, t in enumerate(lines):
            ax.text(
                x + 2.6, y + h - 8.2 - i * 3.5, t, ha="left", va="top", fontsize=7.4
            )

    def link(x1, y1, x2, y2):
        ax.plot(
            [x1, x1, x2],
            [y1, (y1 + y2) / 2, (y1 + y2) / 2],
            color="#8a93a3",
            lw=1.1,
            zorder=0,
        )
        ax.plot([x2, x2], [(y1 + y2) / 2, y2], color="#8a93a3", lw=1.1, zorder=0)

    org = f"{p.get('customer_domain', '')}  ·  Org {p.get('customer_organization_id', '')}"
    box(
        16,
        83,
        68,
        15,
        "Organisation",
        [
            org,
            f"{facts['n']['google_org_policy_policy']} Org-Policies · "
            f"{facts['n']['google_org_policy_custom_constraint']} eigene Constraints · "
            f"{facts['n']['google_organization_iam_member']} IAM-Bindungen",
        ],
        "#eef2fb",
        "#3b5baa",
    )

    fname = (
        facts["folders"][0][0]
        if facts["folders"]
        else p.get("infra_folder_name", "Infrastruktur")
    )
    box(
        30,
        68,
        40,
        10,
        f"Ordner  {fname}",
        ["übergeordnet: Organisation"],
        "#f2f6ee",
        "#5c7a3f",
    )
    link(50, 83, 50, 78)

    xs = [6, 54]
    for i, pr in enumerate(facts["projects"][:2]):
        x = xs[i] if i < len(xs) else 6 + i * 48
        is_infra = pr["id"] == p.get("infra_project_name")
        lines = [f"{len(pr['services'])} aktivierte Services"]
        if is_infra:
            lines += [
                f"State-Bucket: {p.get('infra_bucket_name', '')}",
                f"IaC-SA: {p.get('svc_iac_account', '')}",
            ]
            if facts["topics"]:
                lines += [f"Pub/Sub: {facts['topics'][0]}"]
        else:
            for b in facts["buckets"]:
                if "audit" in b:
                    lines += [f"Log-Archiv: {b}"]
            lines += [f"{len(facts['sinks'])} Organisations-Sinks"]
        box(
            x,
            34,
            40,
            26,
            f"Projekt  {pr['id']}",
            lines,
            "#fdf4ea" if is_infra else "#eef7f7",
            "#a8722c" if is_infra else "#2f6f6f",
        )
        link(50, 68, x + 20, 60)

    box(
        6,
        6,
        88,
        21,
        "Zentrale Ressourcen",
        [
            f"Gruppenmodell: {len(facts['groups'])} Gruppen · {facts['n']['google_cloud_identity_group_membership']} Mitgliedschaften",
            f"Alerting: {facts['n']['google_monitoring_alert_policy']} Alert-Policies über {facts['n']['google_logging_metric']} Log-Metriken",
            f"Abrechnung: {facts['n']['google_billing_account_iam_member']} Bindungen auf dem Billing-Account",
            f"Netzwerk: {facts['n']['google_compute_firewall_policy_rule']} Firewall-Policy-Regeln",
        ],
        "#f6f6f8",
        "#6b7280",
    )
    for x in (26, 74):
        link(x, 34, x, 27)

    # Workloads: der Workload-Ordner (Core-Export `workload_folder`) und die Projekte darunter
    if wide:
        X, W, MID = 102, 28, 116
        top = 78
        if iv["workload_folder"]:
            wname = p.get("workload_folder_name") or "Workloads"
            box(
                X,
                68,
                W,
                10,
                f"Ordner  {wname}",
                [f"{len(wl_projects)} Projekte (Interfaces)"],
                "#f2f6ee",
                "#5c7a3f",
            )
            ax.plot(
                [84, MID, MID], [90.5, 90.5, top], color="#8a93a3", lw=1.1, zorder=0
            )
            top = 68
        else:
            ax.plot([84, MID, MID], [90.5, 90.5, 60], color="#8a93a3", lw=1.1, zorder=0)
        shown = wl_projects[:3]
        for i, pr in enumerate(shown):
            y = 55 - i * 11
            box(
                X,
                y,
                W,
                9.5,
                f"Projekt  {pr['name']}",
                [pr["project_id"] or "Google-Projekt: —"],
                "#eef7f7",
                "#2f6f6f",
            )
            ax.plot([MID, MID], [top, y + 9.5], color="#8a93a3", lw=1.1, zorder=0)
        if len(wl_projects) > 3:
            ax.text(
                MID,
                55 - 3 * 11 + 6,
                f"+{len(wl_projects) - 3} weitere Projekte",
                ha="center",
                va="top",
                fontsize=7.4,
                style="italic",
            )
        if not wl_projects:
            ax.text(
                MID,
                62,
                "noch kein Projekt",
                ha="center",
                va="top",
                fontsize=7.4,
                style="italic",
            )

    fig.tight_layout()
    fig.savefig(png, dpi=190, bbox_inches="tight")
    plt.close(fig)


# ---------- Word ----------------------------------------------------------
PACK_DE = {
    "use_billing_permissions": "Abrechnungsberechtigungen (Billing-Rollen je Gruppe)",
    "use_scc_enablement": "Security Command Center aktiviert",
    "use_audit_logsink": "Organisations-Audit-Sink ins Log-Archiv",
    "use_central_alerts": "Zentrales CIS-Alerting (Log-Metriken + Alert-Policies)",
    "use_essential_contacts": "Essential Contacts je Kategorie",
    "use_budget": "Budget und Budget-Alarme",
    "use_security_audit_sa": "Eigener Audit-Service-Account für Scans",
    "use_defender": "Microsoft Defender for Cloud",
    "use_sentinel": "Microsoft Sentinel",
    "use_verification_runner": "Verification Runner",
    "use_exemption_tag": "Ausnahme-Tag für Org-Policies",
    "use_scc_notifications": "SCC-Benachrichtigungen nach Pub/Sub",
    "use_scc_export": "SCC-Export",
}
GROUP_DE = {
    "gcp-organization-admins": "Organisationsadministration",
    "gcp-project-admins": "Projektadministration",
    "gcp-security-admins": "Sicherheitsadministration (u. a. Security Command Center)",
    "gcp-security-viewers": "Lesender Sicherheitszugang",
    "gcp-billing-admins": "Abrechnung",
    "svc-iac-users": "Darf den IaC-Service-Account impersonieren",
}


COMPANIONS = (
    ("findings", "Checkliste (Einzelbefunde je Kontrolle)", "*Checkliste_{sc}_*.xlsx"),
    (
        "plan",
        "Remediation-Plan (Bewertung und Massnahmen)",
        "Remediation-Plan_{sc}_*.docx",
    ),
)


def gather_companions(shortcode, out):
    """Die Dokumente des Sicherheitsreviews neben das Dokument legen, damit alle Dateien in
    EINEM Verzeichnis liegen und im Text ohne Pfad genannt werden koennen. Jeweils die neueste
    Fassung.

    Diese Dokumente sind Pflicht: Konfigurations-Excel, Kurzbeschreibung, Checkliste und
    Remediation-Plan sind das Standardpaket, das ein Kunde bekommt. Fehlt eines, wird der Lauf
    ABGEBROCHEN, bevor irgendetwas geschrieben wird — ein unvollstaendiges Paket faellt sonst
    erst beim Kunden auf.
    """
    audit = CCC / shortcode / "audit"
    found, missing = {}, []
    for key, label, pattern in COMPANIONS:
        hits = (
            sorted(
                audit.glob(pattern.format(sc=shortcode)),
                key=lambda q: q.stat().st_mtime,
            )
            if audit.is_dir()
            else []
        )
        if not hits:
            missing.append((label, pattern.format(sc=shortcode)))
            continue
        src = hits[-1]
        dst = out / src.name
        if src.resolve() != dst.resolve():
            shutil.copy2(src, dst)
        found[key] = src.name
    if missing:
        lines = ["", "Abbruch: das Standardpaket ist unvollstaendig — es fehlt:", ""]
        for label, pattern in missing:
            lines.append(f"  - {label}")
            lines.append(f"    erwartet als {audit}/{pattern}")
        lines += [
            "",
            "Diese Dokumente gehoeren zu jeder Auslieferung und werden neben die",
            "Kurzbeschreibung kopiert, damit sie im Text ohne Pfad genannt werden koennen.",
            "",
            f"Zuerst das Sicherheitsreview fuer '{shortcode}' erzeugen (Skill security-review),",
            "danach diesen Lauf wiederholen. Es wurde nichts geschrieben.",
            "",
        ]
        sys.exit("\n".join(lines))
    return found


def build_docx(facts, png, out_path, xlsx_name, companions=None):
    from docx import Document
    from docx.enum.text import WD_ALIGN_PARAGRAPH
    from docx.shared import Cm, Pt, RGBColor

    p = facts["params"]
    doc = Document()
    for s in doc.sections:
        s.left_margin = s.right_margin = Cm(2.2)

    doc.add_heading("Cloud-Cockpit — Konfiguration, Kurzbeschreibung", level=0)
    sub = doc.add_paragraph()
    r = sub.add_run(
        f"{p.get('customer_longname', '')} · {p.get('customer_domain', '')}"
    )
    r.bold = True
    r.font.size = Pt(12)
    c = companions or {}
    doc.add_paragraph(
        "Dieses Dokument beschreibt den konfigurierten Zustand, abgeleitet aus dem satz-Estate "
        "(Deklaration und erzeugtes HCL)."
    )
    doc.add_paragraph(f"Die detaillierte Konfiguration findet sich in {xlsx_name}.")
    doc.add_paragraph(
        f"Bewertung und Maßnahmen stehen im Remediation-Plan des Sicherheitsreviews "
        f"({c['plan']}), die Einzelbefunde je Kontrolle in {c['findings']}."
    )

    doc.add_heading("1  Diagramm", level=1)
    doc.add_picture(png, width=Cm(16.4))
    doc.paragraphs[-1].alignment = WD_ALIGN_PARAGRAPH.CENTER
    cap = doc.add_paragraph(
        "Ordner, Projekte und zentrale Ressourcen der Organisation"
        + (
            "; rechts der Workload-Ordner mit den Projekten, die das Interface des Estates lesen."
            if facts["interfaces"]["projects"] or facts["interfaces"]["workload_folder"]
            else "."
        )
    )
    cap.alignment = WD_ALIGN_PARAGRAPH.CENTER
    cap.runs[0].font.size = Pt(8)
    cap.runs[0].font.color.rgb = RGBColor(0x60, 0x60, 0x60)

    def table(rows, head=("Merkmal", "Wert"), widths=None, size=9):
        t = doc.add_table(rows=1, cols=len(head))
        t.style = "Light Grid Accent 1"
        for i, h in enumerate(head):
            t.rows[0].cells[i].text = h
        for r in rows:
            c = t.add_row().cells
            for i, val in enumerate(r):
                c[i].text = str(val)
        if widths:
            for row in t.rows:
                for i, w in enumerate(widths):
                    row.cells[i].width = w
        for row in t.rows:
            for cell in row.cells:
                for par in cell.paragraphs:
                    for run in par.runs:
                        run.font.size = Pt(size)
        doc.add_paragraph()
        return t

    doc.add_heading("2  Wesentliche Fakten", level=1)
    table(
        [
            ("Kunde", p.get("customer_longname", "")),
            ("Primärdomäne", p.get("customer_domain", "")),
            ("Organisations-ID", p.get("customer_organization_id", "")),
            ("Cloud-Identity-Kunden-ID", p.get("customer_id", "")),
            ("Abrechnungskonto", p.get("billing_account_infra", "")),
            (
                "Standardregion / -zone",
                f"{p.get('default_region', '')} / {p.get('default_zone', '')}",
            ),
            (
                "Betriebsmodus",
                f"{p.get('deployment_mode', '')} (State im GCS-Bucket, Impersonation)",
            ),
            ("Werkzeug", p.get("deployment_engine", "")),
        ]
    )

    doc.add_heading("3  Infrastrukturprojekt", level=1)
    doc.add_paragraph(
        f"Das Infrastrukturprojekt {p.get('infra_project_name', '')} trägt die Verwaltung der "
        f"Organisation: den Terraform-State, den IaC-Service-Account und die APIs, über die alle "
        f"weiteren Ressourcen angelegt werden. Alle Provider-Aufrufe laufen mit "
        f"user_project_override gegen dieses Projekt — eine API muss daher hier aktiviert sein, "
        f"nicht dort, wo die Ressource entsteht."
    )
    infra_rows = [
        (
            "Ordner",
            facts["folders"][0][0]
            if facts["folders"]
            else p.get("infra_folder_name", ""),
        ),
        ("Infrastrukturprojekt", p.get("infra_project_name", "")),
        ("State-Bucket", p.get("infra_bucket_name", "")),
        (
            "IaC-Service-Account",
            f"{p.get('svc_iac_account', '')}@{p.get('infra_project_name', '')}.iam.gserviceaccount.com",
        ),
    ]
    for pr in facts["projects"]:
        if pr["id"] != p.get("infra_project_name"):
            infra_rows.append(
                ("Weiteres Projekt", f"{pr['id']} (Log-Archiv und Organisations-Sinks)")
            )
    for b in facts["buckets"]:
        if "audit" in b:
            infra_rows.append(("Log-Archiv-Bucket", b))
    table(infra_rows)

    model = facts.get("security_model")
    doc.add_heading(f"4  Gruppenmodell{' ' + model if model else ''}", level=1)
    doc.add_paragraph(
        "Berechtigungen werden ausschließlich an Gruppen vergeben, nie an einzelne Personen. "
        "Wer eine Rolle braucht, wird Mitglied der zuständigen Gruppe; die Rollenbindung selbst "
        "bleibt unverändert und ist im Estate deklariert. Die Mitgliedschaften sind bewusst NICHT "
        "Teil des Estates — sie gehören dem Kunden."
    )
    grows = []
    for g in facts.get("group_details") or []:
        grows.append(
            (
                f"{g['display_name']}\n{g['email']}",
                g["description"] or GROUP_DE.get(g["email"].split("@")[0], "—"),
                "\n".join(g["roles"]) if g["roles"] else "—",
            )
        )
    if grows:
        table(
            grows,
            head=("Name", "Beschreibung", "Berechtigungen"),
            widths=(Cm(4.2), Cm(5.4), Cm(6.4)),
            size=7.5,
        )
    else:
        table(
            [(g, GROUP_DE.get(g.split("@")[0], "—")) for g in facts["groups"]],
            head=("Gruppe", "Zweck"),
        )
    doc.add_paragraph(
        f"Insgesamt {facts['n']['google_organization_iam_member']} Rollenbindungen auf "
        f"Organisationsebene und {facts['n']['google_billing_account_iam_member']} auf dem "
        f"Abrechnungskonto."
    )

    doc.add_heading("5  Aktivierte Services", level=1)
    doc.add_paragraph(
        f"{len(facts['services'])} APIs sind im Estate deklariert und über "
        f"{facts['n']['google_project_service']} Aktivierungen auf die Projekte verteilt:"
    )
    for s in facts["services"]:
        doc.add_paragraph(s, style="List Bullet")

    doc.add_heading("6  Bausteine (Packs)", level=1)
    doc.add_paragraph("Aktiviert:")
    for k in facts["packs_on"]:
        doc.add_paragraph(f"{PACK_DE.get(k, k)}  ({k})", style="List Bullet")
    if facts["packs_off"]:
        doc.add_paragraph("Nicht aktiviert:")
        for k in facts["packs_off"]:
            doc.add_paragraph(f"{PACK_DE.get(k, k)}  ({k})", style="List Bullet")

    doc.add_heading("7  Compliance-Framework", level=1)
    doc.add_paragraph(
        "Grundlage ist der CIS Google Cloud Foundations Benchmark. Die Kontrollen sind als "
        "Organisationsrichtlinien und Log-basierte Alarme deklariert und wirken präventiv für "
        "jedes neue Projekt."
    )
    table(
        [
            (
                "Organisationsrichtlinien",
                f"{facts['n']['google_org_policy_policy']} Policies",
            ),
            (
                "Eigene Constraints",
                f"{facts['n']['google_org_policy_custom_constraint']}",
            ),
            ("Log-Metriken", f"{facts['n']['google_logging_metric']}"),
            ("Alarmrichtlinien", f"{facts['n']['google_monitoring_alert_policy']}"),
            ("Organisations-Sinks", f"{len(facts['sinks'])}"),
            (
                "Firewall-Policy-Regeln",
                f"{facts['n']['google_compute_firewall_policy_rule']}",
            ),
        ],
        head=("Kontrollart", "Anzahl"),
    )
    doc.add_paragraph(
        "Der Nachweis gegen die laufende Organisation erfolgt mit "
        "satz require und satz report-compliance; die Entscheidungen, die zu dieser "
        f"Konfiguration geführt haben, stehen vollständig in {xlsx_name}."
    )
    doc.add_paragraph(
        f"Der Stand je Kontrolle zum Zeitpunkt des letzten Scans steht in {c['findings']}."
    )

    doc.add_heading("8  Workloads / Projekte", level=1)
    iv, wl = facts["interfaces"], facts["workloads"]
    if not iv["projects"] and not wl:
        doc.add_paragraph(
            "Das Estate veröffentlicht kein Projekt-Interface, und neben dem Estate steht kein Workload."
        )
    else:
        wf = p.get("workload_folder_name") or ""
        doc.add_paragraph(
            "Neben dem Estate stehen die Workloads: Projekte, die das veröffentlichte Interface des "
            "Estates lesen und in eigenen Google-Projekten arbeiten"
            + (f", unter dem Ordner {wf}" if iv["workload_folder"] and wf else "")
            + ". Was ein Projekt liest, steht in seinem Interface (Verzeichnis interfaces/ des Estates); "
            "was es dem Estate anbieten darf, ist als Request-Punkt deklariert und kommt als Pull Request "
            "in das Estate."
        )
        if iv["projects"]:
            offer = ", ".join(q["param"] for q in iv["requests"]) or "—"
            table(
                [
                    (
                        pr["name"],
                        pr["project_id"] or "—",
                        ", ".join(pr["reads"]),
                        offer,
                    )
                    for pr in iv["projects"]
                ],
                head=("Projekt", "Google-Projekt", "Liest", "Darf anbieten"),
                widths=(Cm(3.0), Cm(4.2), Cm(4.4), Cm(4.4)),
                size=8,
            )
        if iv["requests"]:
            doc.add_paragraph(
                "Request-Punkte — die Listen des Estates, zu denen ein Projekt Einträge beitragen darf:"
            )
            table(
                [
                    (
                        q["param"],
                        q["key"],
                        ", ".join(q["fields"]),
                        q["entries"],
                        q["description"],
                    )
                    for q in iv["requests"]
                ],
                head=("Liste", "Schlüssel", "Felder", "Einträge", "Beschreibung"),
                widths=(Cm(3.6), Cm(1.8), Cm(3.6), Cm(1.6), Cm(5.4)),
                size=7.5,
            )
        if wl:
            doc.add_paragraph("Workload-Ordner neben dem Estate:")
            pid_of = {pr["name"]: pr["project_id"] for pr in iv["projects"]}
            table(
                [
                    (
                        w["folder"],
                        KIND_DE.get(w["kind"], w["kind"]),
                        w["interface"] or "—",
                        pid_of.get(w["interface"], "") or "—",
                    )
                    for w in wl
                ],
                head=("Ordner", "Art", "Interface", "Google-Projekt"),
                widths=(Cm(4.4), Cm(4.2), Cm(3.0), Cm(4.4)),
                size=8,
            )
        else:
            doc.add_paragraph("Neben dem Estate steht noch kein Workload-Ordner.")

    doc.save(out_path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--shortcode", required=True)
    ap.add_argument("--repo")
    ap.add_argument("--out-dir")
    ap.add_argument("--index", default="01")
    ap.add_argument("--skip-questions", action="store_true")
    a = ap.parse_args()

    repo = find_repo(a.shortcode, a.repo)
    satz = find_estate(repo)
    out = Path(a.out_dir).expanduser() if a.out_dir else repo.parent
    out.mkdir(parents=True, exist_ok=True)

    companions = gather_companions(a.shortcode, out)
    xlsx = out / f"{a.index}-{a.shortcode}-{satz.stem}-answers.xlsx"
    if not a.skip_questions:
        cmd = [
            "satz",
            "questions",
            satz.name,
            "--config",
            str(repo),
            "--format",
            "xlsx",
            "--out",
            str(xlsx),
        ]
        print("$", " ".join(cmd))
        r = subprocess.run(cmd, capture_output=True, text=True)
        if r.returncode != 0:
            print(r.stdout[-1500:], r.stderr[-1500:], file=sys.stderr)
            sys.exit(f"satz questions schlug fehl ({r.returncode})")

    report = read_interfaces(repo, satz)
    workloads = find_workloads(repo.parent, repo)
    facts = collect(repo, satz, report, workloads)
    docx = out / f"{a.index}-{a.shortcode}-Konfiguration-Kurzbeschreibung.docx"
    with tempfile.TemporaryDirectory() as td:
        png = str(Path(td) / "diagram.png")
        draw(facts, png)
        build_docx(facts, png, str(docx), xlsx.name, companions)

    iv = facts["interfaces"]
    print(f"Estate:   {satz}")
    print(f"Projekte: {', '.join(x['id'] or '?' for x in facts['projects'])}")
    print(
        f"Gruppen:  {len(facts['groups'])} · Services: {len(facts['services'])} · "
        f"Org-Policies: {facts['n']['google_org_policy_policy']}"
    )
    print(
        f"Workloads: {len(iv['projects'])} Projekt-Interface(s) "
        f"({', '.join(pr['name'] + (' · ' + pr['project_id'] if pr['project_id'] else '') for pr in iv['projects']) or '—'}), "
        f"{len(iv['requests'])} Request-Punkt(e), {len(workloads)} Ordner neben dem Estate "
        f"({', '.join(w['folder'] + ':' + w['kind'] for w in workloads) or '—'})"
    )
    print(f"wrote {xlsx}")
    print(f"wrote {docx}")


if __name__ == "__main__":
    main()
