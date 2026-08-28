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
    /// both compose members as `user:{first-admin}@{customer-domain}`, so this must not
    /// carry the domain.
    pub first_admin: String,
}

pub fn generate_template(args: &TemplateArgs, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let content = format!(r#"variables:
  infra-folder-name: &infra-folder-name "Infrastructure"
  infra-project-name: &infra-project-name "{project_id}"
  infra-bucket-name: &infra-bucket-name "{bucket_id}"
  customer-id: &customer-id {customer_id}
  customer-organization-id: &customer-organization-id "{org_id}"
  customer-domain: &customer-domain "{domain}"
  first-admin: &first-admin "{first_admin}"
  customer-longname: &customer-longname ""
  customer-shortname: &customer-shortname "{shortname}"
  svc-iac-account: &svc-iac-account svc-iac-001
  svc-iac-users-group: &svc-iac-users-group svc-iac-users
  billing-account-infra: &billing-account-infra "{billing_id}"
  deployment-engine: &deployment-engine tofu
  deployment-mode: &deployment-mode local # switch by command
  default-region: &default-region {region}
  default-zone: &default-zone {region}-a

terraform:
  backend:
    local:
      path: "terraform.tfstate"
    gcs:
      bucket: *infra-bucket-name
      prefix: "hcl/state"

providers:
  google:
    project: *infra-project-name
    region: *default-region
    alias: google
    user_project_override: true
    billing_project: *infra-project-name
  google-beta:
    project: *infra-project-name
    region: *default-region
    alias: google-beta
    user_project_override: true
    billing_project: *infra-project-name

cloud_identity_group:
  *svc-iac-users-group:
    display_name: Service Account IaC Users
    description: Service account users allowed to impersonate the IaC service account
    owner:
      - !format ["{{}}@{{}}.iam.gserviceaccount.com", *svc-iac-account, *infra-project-name]
    member:
      - !format ["user:{{}}@{{}}", *first-admin, *customer-domain]

google_organization_iam_member:
  # service needs to be added to group admin role in workspace console
  !format ["serviceAccount:{{}}@{{}}.iam.gserviceaccount.com", *svc-iac-account, *infra-project-name]:
    - roles/billing.user
    - roles/billing.projectManager
    - roles/iam.organizationRoleAdmin
    - roles/orgpolicy.policyAdmin
    - roles/owner
    - roles/resourcemanager.folderAdmin
    - roles/resourcemanager.organizationAdmin
    - roles/resourcemanager.projectIamAdmin
    - roles/resourcemanager.projectCreator
    - roles/iam.serviceAccountAdmin
    - roles/serviceusage.serviceUsageAdmin
    - roles/serviceusage.serviceUsageConsumer

  !format ["group:{{}}@{{}}", *svc-iac-users-group, *customer-domain]:
    - roles/iam.serviceAccountTokenCreator
    - roles/iam.serviceAccountUser
    - roles/serviceusage.serviceUsageConsumer

google_billing_account_iam_member:
  billing_account_id: *billing-account-infra
  !format ["serviceAccount:{{}}@{{}}.iam.gserviceaccount.com", *svc-iac-account, *infra-project-name]:
    - roles/billing.admin

folder:
  infra_folder:
    display_name: *infra-folder-name
    project:
      infra:
        project_id: *infra-project-name
        billing_account: *billing-account-infra
        project_service:
          - cloudasset.googleapis.com
          - cloudbilling.googleapis.com
          - cloudidentity.googleapis.com
          - cloudresourcemanager.googleapis.com
          - iam.googleapis.com
          - iamcredentials.googleapis.com
          - logging.googleapis.com
          - orgpolicy.googleapis.com
          - securitycenter.googleapis.com
          - serviceusage.googleapis.com
          - essentialcontacts.googleapis.com

        google_storage_bucket:
          state:
            import-id: *infra-bucket-name
            name: *infra-bucket-name
            location: *default-region
            force_destroy: true
            public_access_prevention: enforced
            uniform_bucket_level_access: true
            lifecycle_rule:
              - action:
                  type: Delete
                condition:
                  num_newer_versions: 100
                  with_state: ARCHIVED
              - action:
                  type: Delete
                condition:
                  days_since_noncurrent_time: 365

        google_service_account:
          provisioner:
            account_id: *svc-iac-account
            display_name: Primary IaC Provisioner

"#,
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-tpl-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    fn args(first_admin: &str, domain: &str) -> TemplateArgs {
        TemplateArgs {
            customer_id: "C09test".into(),
            shortname: "acme".into(),
            billing_id: "0X0X0X-0X0X0X-0X0X0X".into(),
            region: "europe-west3".into(),
            org_id: "123456789012".into(),
            domain: domain.into(),
            project_id: "acme-iac-infra".into(),
            bucket_id: "acme-iac-infra".into(),
            first_admin: first_admin.into(),
        }
    }

    #[test]
    fn admin_membership_uses_anchors_not_a_literal_address() {
        // The generated config must compose the member from *first-admin and
        // *customer-domain, so changing either variable updates every reference.
        // It previously wrote the full address verbatim, which silently diverged
        // from the variables once either was edited.
        let dir = scratch("anchors");
        let path = dir.join("out.yaml");
        generate_template(&args("first.admin", "example.com"), &path).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();

        assert!(
            out.contains(r#"- !format ["user:{}@{}", *first-admin, *customer-domain]"#),
            "member should be built from the anchors:\n{out}"
        );
        assert!(
            !out.contains("first.admin@example.com"),
            "the full address must not appear verbatim:\n{out}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn first_admin_is_emitted_as_an_anchor_holding_only_the_local_part() {
        // Presets and the template both append @customer-domain, so a domain here
        // would render as user:a@example.net@example.net.
        let dir = scratch("localpart");
        let path = dir.join("out.yaml");
        generate_template(&args("first.admin", "example.com"), &path).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();

        assert!(out.contains(r#"first-admin: &first-admin "first.admin""#), "{out}");
        assert!(out.contains(r#"customer-domain: &customer-domain "example.com""#), "{out}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
