---
name: estate-documentation
description: >-
  Erzeugt die Cloud-Cockpit-Kurzdokumentation für eine Kundenorganisation aus dem satz-Estate:
    - Word-Kurzbeschreibung (-de.docx, deutsch,-en.docx, englisch) mit:
      - Abstract und Verweise auf weitere dokumente aus dieser Dokumentation
      - Wesentlichen Konfigurationsparametern (estate-core)
      - Infrastrukturprojekt
      - Gruppenmodell (Gruppen, Permissions)
      - Aktivierte Services und dem
      - Compliance-Framework
      - Workloads / Projekte (aus `satz interfaces`: je Projekt-Interface das Google-Projekt, was es
        liest und was es anbieten darf; die Workload-Ordner neben dem Estate) — inklusive
      - Diagramm der Ordner, Projekte, wichtigen Ressourcen und des Workload-Ordners mit seinen Projekten.
   - Die Entscheidungsliste als Excel (`satz questions --format xlsx`) und pdf (`satz questions --format pdf`)
   - Findings als Excel aus dem Estate Security Review
   - Remediation als Word aus dem Estate Security Review
  Use this skill whenever the user asks for a estate documentation.
  Put the files in the folder under ~/projects/ccc/SHORTCODE/.
  Do not include the path to Documents like ~/projects/ccc/SHORTCODE/ or others as the contain PII.
---

# Estate-Dokumentation für eine Kundenorganisation

Zwei Dateien, nebeneinander, beide aus dem Estate abgeleitet — nichts von Hand gepflegt. Der
Kundenordner (`CCC_ROOT` überschreibt `~/projects/ccc`):

```
~/projects/ccc/<shortcode>/
├── <shortcode>-C<dirid>/                  das ZENTRALE Estate-Repo: config.toml (`yaml_dir` nennt das
│   ├── config.toml                        Estate-Verzeichnis; ohne den Schlüssel liest satz `satz/`),
│   ├── satz/ oder yaml/<id>.satz          darin die .satz und satz/requests/<projekt>.satz (vendorte
│   ├── hcl/  interfaces/  presets/ …      Request-Dateien); hcl/ und interfaces/ erzeugt
├── <workload-ordner>/  (null oder mehr)   ein PROJEKT neben dem Estate: satz-Projekt (config.toml,
│                                          <yaml_dir>/<projekt>.satz mit `use "…/interface.satz"`,
│                                          eigenes hcl/ und vendor/<projekt>/) oder HCL-Projekt
│                                          (.tf-Dateien, die `module.satz.<export>` lesen); alles
│                                          andere daneben (Dokumente, PDFs) ist kein Workload
├── audit/                                 das Sicherheitsreview (Skill security-review)
├── <NN>-<shortcode>-<estate>-Kurzbeschreibung.docx
├── <NN>-<shortcode>-<estate>-Konfiguration.xlsx satz questions --format xlsx
├── <NN>-<shortcode>-<estate>-Konfiguration.pdf  satz questions --format pdf
├── <NN>-<shortcode>-<estate>-CIS-<v>-Checkliste_<date>.xlsx    kopiert aus audit/ (Einzelbefunde)
└── <NN>-<shortcode>-<estate>-Remediation-Plan-<v>_<date>.docx            kopiert aus audit/ (Bewertung)
```

**Ein Verzeichnis, Namen ohne Pfad.** Die beiden Dokumente des Sicherheitsreviews werden aus
`~/projects/ccc/<sc>/audit/` daneben KOPIERT (jeweils die neueste Fassung), damit alle vier Dateien
zusammen liegen und weitergegeben werden können. Im Fliesstext werden sie ausschliesslich mit
ihrem Dateinamen genannt — kein Pfad, keine Verlinkung, kein „siehe Ordner". Auch sonst steht im Dokument kein lokaler Pfad (`/Users/…`, `~/…`, `projects/ccc/…`) — er verrät
Benutzernamen und Kundenablage (PII); Estate und Repo werden nur mit Namen genannt. **Fehlt eines der vier Dokumente, bricht der Lauf ab** und nennt, was fehlt — es wird nichts
geschrieben. Diese vier sind das Standardpaket, das ein Kunde bekommt; ein unvollständiges Paket
fällt sonst erst beim Kunden auf. Bei einem Kunden vor dem ersten Sicherheitsreview also zuerst
den Skill `security-review` laufen lassen, dann diesen.

Der einleitende Absatz lautet immer:

> Dieses Dokument beschreibt die Konfiguration der Google Cloud Umgebung,
> abgeleitet aus dem satz-Estate (Deklaration und erzeugtes HCL).
> Die detaillierten einstellungen finden sich in <NN>-<shortcode>-<estate>-Konfiguration.xlsx.
> Bewertung und Maßnahmen stehen im Remediation-Plan des Sicherheitsreviews <NN>-<shortcode>-<estate>-Remediation-Plan-<v>_<date>.docx,
> die Einzelbefunde je Control in <NN>-<shortcode>-<estate>-CIS-<v>-Checkliste_<date>.xlsx.

`<NN>` ist ein zweistelliger Index (Vorgabe `01`), damit die Dokumente eines Kunden in der
Reihenfolge ihrer Entstehung sortieren.

## Ablauf

Ein Aufruf, alles offline ausser `satz questions`:

```
uv run --with python-docx --with matplotlib python3 $SKILL/scripts/build_estate_documentation.py \
    --shortcode <sc> [--index 01] [--out-dir <dir>]
```

Das Skript
1. findet das Estate-Repo `~/projects/ccc/<sc>/<sc>-C<dirid>/` und die `.satz`-Datei im
   `yaml_dir` seiner `config.toml` (ohne den Schlüssel: `satz/`),
2. ruft `satz questions <estate> --config <repo> --format xlsx --out <out>/<NN>-<sc>-<estate>-answers.xlsx`
   (ADR 0021: `--format` und `--out` sind beide Pflicht, genau eine Datei je Lauf),
3. ruft `satz interfaces <estate> --config <repo> --format json --out -` (offline: kompiliert das
   Estate im Speicher) — jedes Export mit seinem Interface, jedes Interface, jeder Request-Punkt;
   schlägt das fehl, bricht der Lauf ab — und findet die Workload-Ordner neben dem Repo,
4. liest die Fakten aus dem Estate — `params { … }` der `.satz` und die erzeugte `hcl/main.tf` —,
5. zeichnet das Diagramm (Organisation → Ordner → Projekte → wichtige Ressourcen; rechts der
   Workload-Ordner, Core-Export `workload_folder`, mit den Projekt-Interfaces darunter) als PNG,
6. schreibt die Word-Kurzbeschreibung mit Diagramm, Faktentabelle, Gruppenmodell, Services,
   Org-Policies, Compliance-Framework und dem Abschnitt „Workloads / Projekte".

Danach das Dokument selbst lesen und die Zahlen gegen das Estate prüfen. Alles, was im Dokument
steht, muss aus `.satz`, `main.tf` oder `satz interfaces` stammen; steht eine Angabe in keiner
dieser Quellen, gehört sie nicht hinein.

## Quellen, und was daraus wird

| Abschnitt | Quelle |
|---|---|
| Kopf, Fakten | `params { … }`: `customer_longname`, `customer_domain`, `customer_organization_id`, `customer_id`, `billing_account_infra`, `default_region`, `deployment_mode`, `deployment_engine` |
| Infrastrukturprojekt | `infra_project_name`, `infra_bucket_name`, `svc_iac_account`, `google_project`/`google_folder` in `main.tf` |
| Gruppenmodell | `security_model_*`-Param für den Modellnamen (S1, S2 …); je Gruppe `display_name`, `description` und `group_key.id` aus `google_cloud_identity_group`, die Rollen aus `google_organization_iam_member` und `google_billing_account_iam_member` (`member = "group:…"`). Tabelle Name / Beschreibung / Berechtigungen, ohne erklärenden Fliesstext — die Rollen sind das, was tatsächlich gebunden IST, nicht was die Vorlage vorsieht |
| Services | `google_project_service` in `main.tf`, je Projekt |
| Org-Policies, Constraints | `google_org_policy_policy`, `google_org_policy_custom_constraint` |
| Logging, Alerting, SCC | `google_logging_organization_sink`, `google_logging_metric`, `google_monitoring_alert_policy`, `google_pubsub_topic`, `google_storage_bucket` |
| Compliance-Framework | `use_*`-Params (aktivierte Packs) und die `claim`-Zeilen der Packs |
| Workloads / Projekte | `satz interfaces <estate> --format json --out -`: je Projekt-Interface (nicht `common`) das Google-Projekt aus dem statischen Export `project_id`, „Liest" = core, die gemeinsamen Interfaces und `uses`, „Darf anbieten" = die Request-Punkte (`contributes_<param>`, Schlüssel, Felder, Einträge); der Workload-Ordner aus dem Core-Export `workload_folder` und `workload_folder_name`; die Ordner neben dem Repo mit ihrer Art (satz-Projekt: `config.toml` + `.satz` mit `use "…/interface.satz"`; HCL-Projekt: `.tf` mit `module.satz.<export>`) — nur mit Namen, nie mit Pfad |

## Regeln

- **Deutsch im deutschsprachigen Fliesstext, englische Bezeichner unverändert** — Projekt-IDs, Gruppenadressen,
  Rollen, Constraint-Namen und Kommandos wörtlich.
- **Nur Abgeleitetes.** Keine Bewertung, keine Empfehlung: das ist der Remediation-Plan der
  `security-review`-Skill. Hier steht, was konfiguriert IST.
- **Reproduzierbar.** Änderungen gehen ins Estate und werden neu erzeugt, nie ins .docx.
- **`.docx`, nicht `.doc`** — python-docx schreibt Office Open XML; eine Datei mit `.doc`-Endung
  und `.docx`-Inhalt lässt Word warnen. Beim Umbenennen bleibt das Format, nur die Endung lügt.
