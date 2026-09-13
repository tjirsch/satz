//! The estate `satz init --customer-id` writes: the Day-0 skeleton `bootstrap`
//! expects (its imports are addressed by label — `google_folder.infra_folder`,
//! `google_project.infra`, `google_storage_bucket.state` — so those labels are
//! a contract, not a style). Written in Satz: a new customer's first file must
//! be one every command accepts.
//!
//! The contract reaches further than `bootstrap` now: the CIS pack claims CIS 5.0
//! §2.14 (Cloud Asset Inventory enabled) against
//! `google_project_service.infra_cloudasset_googleapis_com`, which the emitter derives
//! from the `infra` project label and the service name below. Renaming the label, or
//! dropping `cloudasset.googleapis.com` from that list, turns a control every estate
//! satisfies into a broken claim. `the_generated_estate_compiles_and_carries_bootstraps_labels`
//! holds both.

use std::path::Path;

pub struct TemplateArgs {
    pub customer_id: String,
    pub shortname: String,
    pub billing_id: String,
    pub region: String,
    pub org_id: String,
    pub domain: String,
    pub project_id: String,
    pub bucket_id: String,
    /// Local part of the initial admin address. The template and the shipped presets
    /// both compose members as `user:{first_admin}@{customer_domain}`, so this must not
    /// carry the domain.
    pub first_admin: String,
}

/// The estate name: the customer id as a Satz identifier.
pub(crate) fn estate_name(customer_id: &str) -> String {
    let mut s: String = customer_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect();
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// Everything an estate carries besides its params: the backend, the providers,
/// the IaC group and service account, the management folder, project and state
/// bucket — the labels `bootstrap` imports by name. Shared by `init` and the
/// interview skeleton, so the two ways to start end at the same file.
pub(crate) const SCAFFOLD: &str = r#"terraform {
  backend {
    local { path = "terraform.tfstate" }
    gcs {
      bucket = infra_bucket_name
      prefix = "hcl/state"
    }
  }
}

providers {
  "google" {
    project               = infra_project_name
    region                = default_region
    alias                 = "google"
    user_project_override = true
    billing_project       = infra_project_name
  }
  "google-beta" {
    project               = infra_project_name
    region                = default_region
    alias                 = "google-beta"
    user_project_override = true
    billing_project       = infra_project_name
  }
}

google_cloud_identity_group {
  svc_iac_users {
    id           = "{svc_iac_users_group}@{customer_domain}"
    display_name = "Service Account IaC Users"
    description  = "Service account users allowed to impersonate the IaC service account"
    owner        = ["{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com"]
    member       = ["user:{first_admin}@{customer_domain}"]
  }
}

google_organization_iam_member {
  // The IaC service account: read on every project (import, adopt, reports), and
  // the roles this estate's resource types need — `satz iac-roles` adds the ones
  // further packs bring. Granted at the organization, so every folder and project
  // inherits them, hand-made ones included. The Groups Admin role in the Workspace
  // admin console is needed as well; it is not an IAM role.
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/viewer",
    "roles/browser",
    "roles/iam.securityReviewer",
    "roles/cloudasset.viewer",
    "roles/serviceusage.serviceUsageConsumer",
    "roles/resourcemanager.organizationAdmin",
    "roles/orgpolicy.policyAdmin",
    "roles/resourcemanager.folderAdmin",
    "roles/resourcemanager.projectCreator",
    "roles/resourcemanager.projectMover",
    "roles/billing.projectManager",
    "roles/serviceusage.serviceUsageAdmin",
    "roles/iam.serviceAccountAdmin",
    "roles/storage.admin",
  ]
  "group:{svc_iac_users_group}@{customer_domain}" = [
    "roles/serviceusage.serviceUsageConsumer",
  ]
}

// The users group may become the IaC service account — that account only, not
// every service account in the organization.
google_service_account_iam_member {
  service_account_id = "${{google_service_account.provisioner.name}}"
  "group:{svc_iac_users_group}@{customer_domain}" = [
    "roles/iam.serviceAccountTokenCreator",
    "roles/iam.serviceAccountUser",
  ]
}

google_billing_account_iam_member {
  billing_account_id = billing_account_infra
  "serviceAccount:{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" = [
    "roles/billing.admin",
  ]
}

google_folder {
  infra_folder {
    display_name = infra_folder_name
    google_project {
      infra {
        project_id      = infra_project_name
        billing_account = billing_account_infra
        project_service = [
          "cloudasset.googleapis.com",
          "cloudbilling.googleapis.com",
          "cloudidentity.googleapis.com",
          "cloudresourcemanager.googleapis.com",
          "iam.googleapis.com",
          "iamcredentials.googleapis.com",
          "logging.googleapis.com",
          "orgpolicy.googleapis.com",
          "securitycenter.googleapis.com",
          "securitycentermanagement.googleapis.com",
          "serviceusage.googleapis.com",
          "essentialcontacts.googleapis.com",
        ]
        google_storage_bucket {
          state {
            "import-id"                 = infra_bucket_name
            name                        = infra_bucket_name
            location                    = default_region
            force_destroy               = true
            public_access_prevention    = "enforced"
            uniform_bucket_level_access = true
            lifecycle_rule = [
              {
                action { type = "Delete" }
                condition { num_newer_versions = 100 with_state = "ARCHIVED" }
              },
              {
                action { type = "Delete" }
                condition { days_since_noncurrent_time = 365 }
              },
            ]
          }
        }
        google_service_account {
          provisioner {
            account_id   = svc_iac_account
            display_name = "Primary IaC Provisioner"
          }
        }
      }
    }
  }
}
"#;

/// Every pack line an estate can carry, the param that gates it, and the PHASE that has to
/// be finished before it can go in — in the order they can be adopted.
///
/// One table, two writers, so they cannot disagree: `skeleton` writes the whole menu when
/// an estate is created, and `merge-presets` appends the line for a pack the library has
/// gained since (before this table, nobody wrote that line — the map would declare a new
/// choice, `satz questions` would ask it, and answering yes did nothing at all, silently).
/// A hand-written line lands somewhere different every time; a written one is uniform.
///
/// A phase repeated on consecutive rows is printed once, so the menu reads as blocks. A map
/// choice with no row here fails `the_map_and_the_skeleton_stay_equal`, which is what keeps
/// a new pack from reaching the library without anyone saying when it can be adopted.
pub(crate) const PACK_LINES: &[(&str, &str, &str)] = &[
    (
        "presets/estate-map.satz",
        "",
        "once the estate runs as the service account — the map, which declares the questions\n\
         // every line below is gated on. This is the one to uncomment first.",
    ),
    (
        "presets/security-group-models/s1-security-groups.satz",
        "security_model_s1",
        "once the map is in — the security-group model, whose groups later grants name.\n\
         // Exactly one of the two.",
    ),
    ("presets/security-group-models/s2-security-groups.satz", "security_model_s2", ""),
    (
        "presets/billing-account-permissions.satz",
        "use_billing_permissions",
        "once the groups exist — this grants to the model's billing-admins group by name, so\n\
         // the grant has nothing to land on until they are applied",
    ),
    (
        "presets/organization-budget.satz",
        "use_budget",
        "once the estate runs as the service account — these three stand alone",
    ),
    ("presets/security-audit/sa-security-audit.satz", "use_security_audit_sa", ""),
    ("presets/scc/scc-service-enablement.satz", "use_scc_enablement", ""),
    (
        "presets/cis-extensions/block-project-ssh-keys.satz",
        "cis_block_project_ssh_keys",
        "once the CIS baseline is in — its extensions are gated on params the baseline\n\
         // declares, so they do not compile without it. Each restricts what may be created;\n\
         // two are on by default (DNS query logging, and the admin ports closed to the\n\
         // internet).",
    ),
    ("presets/cis-extensions/shielded-vm.satz", "cis_require_shielded_vm", ""),
    ("presets/cis-extensions/dns-logging.satz", "cis_dns_logging", ""),
    ("presets/cis-extensions/confidential-computing.satz", "cis_confidential_computing", ""),
    ("presets/cis-extensions/cloud-sql.satz", "cis_cloud_sql_hardening", ""),
    ("presets/cis-extensions/cmek.satz", "cis_cmek_required", ""),
    ("presets/cis-extensions/api-key-services.satz", "cis_api_key_services", ""),
    ("presets/cis-extensions/bucket-retention.satz", "cis_bucket_retention", ""),
    ("presets/cis-extensions/access-approval.satz", "cis_access_approval", ""),
    ("presets/cis-extensions/internet-ssh-rdp.satz", "cis_block_internet_ssh_rdp", ""),
    (
        "presets/cis-extensions/cloud-sql-iam-and-deletion-protection.satz",
        "cis_cloud_sql_iam_and_deletion_protection",
        "",
    ),
    (
        "presets/cis-extensions/api-key-services-dry-run.satz",
        "cis_api_key_services_dry_run",
        "INSTEAD of the enforcing extension above it, never beside it. A dry run declares the\n\
         // same policy with `dry_run_spec`: Google logs every action it would have blocked and\n\
         // blocks none, so the violation count sizes the control against this organisation\n\
         // before it bites. Nothing is enforced while it runs.",
    ),
    (
        "presets/cis-extensions/block-project-ssh-keys-dry-run.satz",
        "cis_block_project_ssh_keys_dry_run",
        "",
    ),
    ("presets/cis-extensions/bucket-retention-dry-run.satz", "cis_bucket_retention_dry_run", ""),
    ("presets/cis-extensions/cloud-sql-dry-run.satz", "cis_cloud_sql_hardening_dry_run", ""),
    (
        "presets/cis-extensions/cloud-sql-iam-and-deletion-protection-dry-run.satz",
        "cis_cloud_sql_iam_and_deletion_protection_dry_run",
        "",
    ),
    (
        "presets/cis-extensions/confidential-computing-dry-run.satz",
        "cis_confidential_computing_dry_run",
        "",
    ),
    (
        "presets/scc/scc-notifications.satz",
        "use_scc_notifications",
        "once Security Command Center is switched on — findings have to exist before anything\n\
         // can carry them",
    ),
    ("presets/scc/scc-export.satz", "use_scc_export", ""),
    (
        "presets/scc/scc-findings-siem.satz",
        "use_scc_findings_siem",
        "once the findings topic exists — this reads from it",
    ),
    (
        "presets/scc/scc-findings-mail.satz",
        "use_scc_findings_mail",
        "once the central alerts are in as well — the mailbox defaults to their address",
    ),
    (
        "presets/integrations/microsoft-defender-for-cloud.satz",
        "use_defender",
        "once the estate runs as the service account — Defender's plan fragments are added by\n\
         // hand once this line is in; see that pack's header",
    ),
    (
        "presets/integrations/microsoft-sentinel.satz",
        "use_sentinel",
        "once the audit archive exists — Sentinel's project defaults to the logsink's",
    ),
    (
        "presets/integrations/microsoft-sentinel-auditlogs.satz",
        "use_sentinel_auditlogs",
        "once Sentinel's federation is in — these read as the account it creates",
    ),
    ("presets/integrations/microsoft-sentinel-network-logs.satz", "use_sentinel_network_logs", ""),
    (
        "presets/ci/verification-runner.satz",
        "use_verification_runner",
        "once the estate runs as the service account — the runner, then the grant that trusts\n\
         // it by naming the runner's own account",
    ),
    ("presets/ci/verification-runner-grant.satz", "use_verification_runner", ""),
    (
        "presets/exemptions/exemption-tag.satz",
        "use_exemption_tag",
        "any time after the organisation policies — it creates the tag an exemption is bound to\n\
         // and exempts nothing on its own. The BINDING that lets one resource out, and the\n\
         // condition on the constraint that honours it, are the estate's to write; the pack's\n\
         // header shows both",
    ),
];

/// The commented menu, as the skeleton writes it: one line per pack, a phase comment above
/// each group, and the whole thing inert until a line is uncommented.
pub(crate) fn pack_menu() -> String {
    let mut out = String::from(
        "// ---- the packs, each under the phase that comes before it ----------------------\n\
         //\n\
         // Day 0 is the scaffold alone. Bootstrap it, apply it, then `satz migrate --mode cloud`\n\
         // so the state and the identity move to the IaC service account — and only then does a\n\
         // pack go in, one at a time, each with its own plan and apply. That is why every line\n\
         // below is commented out: an estate that used four packs on day 0 could not be applied\n\
         // until somebody had answered for packs nobody had chosen yet.\n\
         //\n\
         // Uncomment a line to add its pack. `satz interview` does it when that pack's question\n\
         // is answered yes, and `satz merge-presets` adds the line for a pack the library has\n\
         // gained since — so the list here stays the library's, not one person's memory of it.\n\
         // Whichever writes it, the compile reports a question answered true whose line is still\n\
         // commented, so the two never drift apart.\n",
    );
    for (path, gate, phase) in PACK_LINES {
        if !phase.is_empty() {
            out.push_str(&format!("\n// {}\n", phase));
        }
        out.push_str(&pack_line(path, gate));
        out.push('\n');
    }
    out
}

/// One commented line, exactly as both writers must write it.
pub(crate) fn pack_line(path: &str, gate: &str) -> String {
    if gate.is_empty() {
        format!("// use \"{}\"", path)
    } else {
        format!("// use \"{}\" when {}", path, gate)
    }
}

/// The estate an INTERVIEW starts from: the day-0 scaffold, and every pack commented out.
///
/// The day-0 params and their questions come from `presets/estate-core.satz`, and that is
/// the only `use` the file starts with — sixteen questions, all of them about the estate
/// itself. Every pack line is written COMMENTED, under the phase that has to be finished
/// before it can go in, because the real order of work is: bootstrap, apply, move the
/// state and the identity to the service account with `satz migrate`, and only then add
/// packs one at a time. A day-0 file that already used four packs could not be applied
/// until somebody answered for packs they had not chosen yet.
///
/// Uncommenting a line is what adds its pack. `satz interview` does it when the pack's
/// question is answered yes; an operator or an agent can do it by hand. Whichever writes
/// it, the compile is what keeps them honest: a pack's question answered true while its
/// line is still commented is reported, never silently ignored.
///
/// `presets/estate-map.satz` is the line to uncomment first — it is what declares the
/// questions the other lines are gated on.
pub(crate) fn skeleton(stem: &str) -> String {
    let scaffold = SCAFFOLD.replacen(
        "    display_name = infra_folder_name\n",
        "    display_name = infra_folder_name\n\
         \x20   // once the estate runs as the service account — the audit archive, which every\n\
         \x20   // later logging pack points at\n\
         \x20   // use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink\n\
         \x20   // once the archive exists — the alert project defaults to the logsink's, so this\n\
         \x20   // does not compile without it\n\
         \x20   // use \"presets/monitoring/organization-cis-log-alerts-central.satz\" when use_central_alerts\n",
        1,
    );
    debug_assert!(scaffold != SCAFFOLD, "the folder anchor the skeleton hangs the logging packs on is gone");
    format!(
        r#"// Written for an interview: a question is open until its param is bound below.
// `satz questions {stem}.satz --unanswered` lists what is still to decide;
// bootstrap and apply refuse until nothing is.

estate {estate}

params {{
}}

// The day-0 params and their questions. The only pack the file starts with: sixteen
// questions, every one of them about the estate itself.
use "presets/estate-core.satz"

{menu}
// The CIS baseline. Not a choice — it is what the estate is for — but it is also thirty
// organisation policies, so it goes in deliberately, after the switch to the service
// account, with its own plan read before it is applied.
google_org_policy_policy {{
  // use "presets/CIS-GCP-Foundation-4.0.satz"
}}

// once the estate runs as the service account — one contact for Google's notices
google_essential_contacts_contact {{
  // use "presets/essential-contacts-organization.satz" when use_essential_contacts
}}

{scaffold}"#,
        stem = stem,
        estate = estate_name(stem),
        scaffold = scaffold,
        menu = pack_menu(),
    )
}

pub fn generate_template(args: &TemplateArgs, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = format!(
        r#"// Generated by `satz init` — the Day-0 estate `satz bootstrap` builds on.
// Every value below is a param: change it here, never in a pack.

estate {estate}

params {{
  infra_folder_name        = "Infrastructure"
  infra_project_name       = "{project_id}"
  infra_bucket_name        = "{bucket_id}"
  customer_id              = "{customer_id}"
  customer_organization_id = "{org_id}"
  customer_domain          = "{domain}"
  first_admin              = "{first_admin}"
  customer_longname        = ""
  customer_shortname       = "{shortname}"
  svc_iac_account          = "svc-iac-001"
  svc_iac_users_group      = "svc-iac-users"
  billing_account_infra    = "{billing_id}"
  deployment_engine        = "tofu"
  deployment_mode          = "local" // switched by `satz migrate`
  default_region           = "{region}"
  default_zone             = "{region}-a"
}}

{scaffold}"#,
        scaffold = SCAFFOLD,
        estate = estate_name(&args.customer_id),
        customer_id = args.customer_id,
        project_id = args.project_id,
        bucket_id = args.bucket_id,
        org_id = args.org_id,
        domain = args.domain,
        first_admin = args.first_admin,
        shortname = args.shortname,
        billing_id = args.billing_id,
        region = args.region,
    );

    crate::fsx::write_generated_satz(output_path, &content)?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-tpl-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    pub(crate) fn args(first_admin: &str, domain: &str) -> TemplateArgs {
        TemplateArgs {
            customer_id: "C0example".into(),
            shortname: "acme".into(),
            billing_id: "012345-6789AB-CDEF01".into(),
            region: "europe-west3".into(),
            org_id: "123456789012".into(),
            domain: domain.into(),
            project_id: "acme-iac-infra".into(),
            bucket_id: "acme-iac-infra".into(),
            first_admin: first_admin.into(),
        }
    }

    /// The templates are canonical at the source: what `init` and the interview write
    /// is what the formatter would write, so the source reads as the file does. A
    /// failure here is fixed in the template, never in the test.
    #[test]
    fn the_skeleton_is_in_the_canonical_layout() {
        let sk = skeleton("acme");
        assert_eq!(satz_core::fmt::format(&sk).unwrap(), sk, "template::skeleton is not formatted");
    }

    #[test]
    fn the_init_estate_is_in_the_canonical_layout() {
        let dir = scratch("canon");
        let path = dir.join("C0example.satz");
        generate_template(&args("first.admin", "example.com"), &path).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert_eq!(satz_core::fmt::format(&out).unwrap(), out);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Only a `use` line gates a pack. Prose in a comment may say the word "when" and
    /// mean nothing by it, so the scan reads the lines that are lines.
    fn use_lines(src: &str) -> impl Iterator<Item = &str> {
        src.lines().map(str::trim).filter(|l| l.starts_with("use ") || l.starts_with("// use "))
    }

    #[test]
    fn the_skeleton_carries_one_use_line_per_choice_the_map_declares() {
        // Two places for one list — the map declares the choices, the skeleton carries their
        // `use … when` lines. This is what keeps them equal.
        let map = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/estate-map.satz")).unwrap();
        let map = satz_core::satz::parse(&map).unwrap();
        let sk = skeleton("x");
        for (name, _, _) in &map.params {
            assert!(
                use_lines(&sk).any(|l| l.ends_with(&format!(" when {}", name))),
                "the map declares `{}` and the skeleton has no `use … when {}`",
                name,
                name
            );
        }
        for line in use_lines(&sk).filter(|l| l.contains(" when ")) {
            let param = line.rsplit(" when ").next().unwrap().trim();
            let declared = map.params.iter().any(|(n, _, _)| n == param) || param.starts_with("cis_");
            assert!(declared, "the skeleton gates a pack on `{}`, which neither the map nor the CIS baseline declares", param);
        }
        assert!(sk.contains("use \"presets/estate-map.satz\"\n"));
        assert!(
            sk.contains("    // use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink\n"),
            "the logging packs sit in the infrastructure folder, commented like every other pack"
        );
    }

    #[test]
    fn every_map_choice_has_a_phase_and_the_menu_is_inert() {
        // The table is what `merge-presets` inserts from, so a pack the library gains without
        // a row would be a pack nobody can adopt — the map would ask for it and no line would
        // ever be written. This is that gate.
        let map = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/estate-map.satz")).unwrap();
        let map = satz_core::satz::parse(&map).unwrap();
        let gated: Vec<&str> = PACK_LINES.iter().map(|(_, gate, _)| *gate).collect();
        for (name, _, _) in &map.params {
            // the two logging packs live inside the folder, written by the scaffold
            if name == "use_audit_logsink" || name == "use_central_alerts" || name == "use_essential_contacts" {
                continue;
            }
            assert!(gated.contains(&name.as_str()), "the map declares `{}` and PACK_LINES has no row saying when it can be adopted", name);
        }
        // and the menu enforces nothing until a line is uncommented
        let sk = skeleton("x");
        for line in use_lines(&sk).filter(|l| !l.contains("estate-core")) {
            assert!(line.starts_with("// use "), "a pack line in a fresh skeleton must be commented: {}", line);
        }
        // the day-0 pack is the exception: it is not in the menu at all
        assert!(sk.contains("\nuse \"presets/estate-core.satz\"\n"), "estate-core is the one pack a day-0 file uses");
    }

    #[test]
    fn members_are_composed_from_params_not_written_verbatim() {
        // Changing `first_admin` or `customer_domain` must update every reference;
        // the old YAML template once wrote the full address and silently diverged.
        let dir = scratch("params");
        let path = dir.join("out.satz");
        generate_template(&args("first.admin", "example.com"), &path).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#"member       = ["user:{first_admin}@{customer_domain}"]"#), "{out}");
        assert!(!out.contains("first.admin@example.com"), "{out}");
        assert!(out.contains(r#"first_admin              = "first.admin""#), "{out}");
        assert!(out.contains(r#"customer_domain          = "example.com""#), "{out}");
        assert!(out.contains("estate c0example"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn estate_names_are_identifiers() {
        assert_eq!(estate_name("C0example"), "c0example");
        assert_eq!(estate_name("0abc-x"), "_0abc_x");
    }
}
