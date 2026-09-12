//! The estate `satz init --customer-id` writes: the Day-0 skeleton `bootstrap`
//! expects (its imports are addressed by label — `google_folder.infra_folder`,
//! `google_project.infra`, `google_storage_bucket.state` — so those labels are
//! a contract, not a style). Written in Satz: a new customer's first file must
//! be one every command accepts.

use std::fs;
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
    owner        = [ "{svc_iac_account}@{infra_project_name}.iam.gserviceaccount.com" ]
    member       = [ "user:{first_admin}@{customer_domain}" ]
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
              { action { type = "Delete" }
                condition { num_newer_versions = 100 with_state = "ARCHIVED" } },
              { action { type = "Delete" }
                condition { days_since_noncurrent_time = 365 } },
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

/// The estate an INTERVIEW starts from: every question open, nothing decided.
///
/// The day-0 params and their questions come from `presets/estate-core.satz`;
/// which packs make up the estate is `presets/estate-map.satz`, whose every
/// choice is one `use … when` line here, in the map's order — a pack switched
/// on brings its own questions with it. The CIS baseline is not a choice. The
/// resources are the scaffold `init` writes, with the logging packs placed in
/// the infrastructure folder beside the infrastructure project. Answering a
/// question is adding its param to `params {}`; the file is complete when
/// `satz questions` says so, and until then `bootstrap` and `transpile --apply`
/// refuse it.
pub(crate) fn skeleton(stem: &str) -> String {
    let scaffold = SCAFFOLD.replacen(
        "    display_name = infra_folder_name\n",
        "    display_name = infra_folder_name\n\
         \x20   use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink\n\
         \x20   use \"presets/monitoring/organization-cis-log-alerts-central.satz\" when use_central_alerts\n",
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

// The day-0 params with their questions, then the map: which packs, as questions.
use "presets/estate-core.satz"
use "presets/estate-map.satz"

// The CIS baseline is not a choice — it is what the estate is for. Its opt-in
// extensions are the baseline pack's own questions.
google_org_policy_policy {{
  use "presets/CIS-GCP-Foundation-4.0.satz"
}}
use "presets/cis-extensions/block-project-ssh-keys.satz" when cis_block_project_ssh_keys
use "presets/cis-extensions/shielded-vm.satz" when cis_require_shielded_vm
use "presets/cis-extensions/dns-logging.satz" when cis_dns_logging
use "presets/cis-extensions/confidential-computing.satz" when cis_confidential_computing
use "presets/cis-extensions/cloud-sql.satz" when cis_cloud_sql_hardening
use "presets/cis-extensions/cmek.satz" when cis_cmek_required
use "presets/cis-extensions/api-key-services.satz" when cis_api_key_services
use "presets/cis-extensions/bucket-retention.satz" when cis_bucket_retention
use "presets/cis-extensions/access-approval.satz" when cis_access_approval
use "presets/cis-extensions/internet-ssh-rdp.satz" when cis_block_internet_ssh_rdp
use "presets/cis-extensions/cloud-sql-iam-and-deletion-protection.satz" when cis_cloud_sql_iam_and_deletion_protection

// The map's choices, one line each. The audit archive and the central alerts are
// in the infrastructure folder below, beside the infrastructure project.
use "presets/security-group-models/s1-security-groups.satz" when security_model_s1
use "presets/security-group-models/s2-security-groups.satz" when security_model_s2
use "presets/billing-account-permissions.satz" when use_billing_permissions
use "presets/organization-budget.satz" when use_budget
use "presets/scc/scc-service-enablement.satz" when use_scc_enablement
use "presets/scc/scc-notifications.satz" when use_scc_notifications
use "presets/scc/scc-findings-mail.satz" when use_scc_findings_mail
use "presets/scc/scc-findings-siem.satz" when use_scc_findings_siem
use "presets/scc/scc-export.satz" when use_scc_export
use "presets/security-audit/sa-security-audit.satz" when use_security_audit_sa
use "presets/ci/verification-runner.satz" when use_verification_runner
use "presets/ci/verification-runner-grant.satz" when use_verification_runner
// Defender's plan fragments are added by hand once this is true — see that pack's header.
use "presets/integrations/microsoft-defender-for-cloud.satz" when use_defender
use "presets/integrations/microsoft-sentinel.satz" when use_sentinel
use "presets/integrations/microsoft-sentinel-auditlogs.satz" when use_sentinel_auditlogs
use "presets/integrations/microsoft-sentinel-network-logs.satz" when use_sentinel_network_logs

google_essential_contacts_contact {{
  use "presets/essential-contacts-organization.satz" when use_essential_contacts
}}

{scaffold}"#,
        stem = stem,
        estate = estate_name(stem),
        scaffold = scaffold,
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

    fs::write(output_path, content)?;
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

    #[test]
    fn the_skeleton_carries_one_use_line_per_choice_the_map_declares() {
        // Two places for one list — the map declares the choices, the skeleton
        // carries their `use … when` lines. This is what keeps them equal.
        let map = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/estate-map.satz")).unwrap();
        let map = satz_core::satz::parse(&map).unwrap();
        let sk = skeleton("x");
        for (name, _, _) in &map.params {
            assert!(sk.contains(&format!(" when {}\n", name)), "the map declares `{}` and the skeleton has no `use … when {}`", name, name);
        }
        for line in sk.lines().filter(|l| l.contains(" when ")) {
            let param = line.rsplit(" when ").next().unwrap().trim();
            let declared = map.params.iter().any(|(n, _, _)| n == param) || param.starts_with("cis_");
            assert!(declared, "the skeleton gates a pack on `{}`, which neither the map nor the CIS baseline declares", param);
        }
        assert!(sk.contains("use \"presets/estate-map.satz\"\n"));
        assert!(sk.contains("    use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink\n"), "the logging packs sit in the infrastructure folder");
    }

    #[test]
    fn members_are_composed_from_params_not_written_verbatim() {
        // Changing `first_admin` or `customer_domain` must update every reference;
        // the old YAML template once wrote the full address and silently diverged.
        let dir = scratch("params");
        let path = dir.join("out.satz");
        generate_template(&args("first.admin", "example.com"), &path).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#"member       = [ "user:{first_admin}@{customer_domain}" ]"#), "{out}");
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
