//! Bootstrap pre-flight: test the caller's REQUIRED PERMISSIONS before
//! anything is created, and self-grant the missing ones when the caller holds
//! `setIamPolicy` on the scope root.
//!
//! Permissions, never roles: roles are how permissions arrive, not what
//! bootstrap needs. The real-world case this exists for is a freshly
//! materialized organization — everyone in the domain holds projectCreator +
//! billing.creator, the creating super admin holds organizationAdmin, and
//! organizationAdmin carries `organizations.setIamPolicy` but NOT
//! `folders.create` / `orgpolicy.policies.create` — so the old bootstrap
//! 403'd mid-run on the org's own admin. Now the missing pair
//! (folderAdmin + orgpolicy.policyAdmin) is granted up front, audibly, or the
//! exact grant commands are printed and nothing is created.
//!
//! Folder-scoped installs (`customer_organization_id = "folders/<id>"`) are
//! first-class: org-root operations are simply out of scope and say so — a
//! folder-granted caller is never told to become org admin.

use crate::gcp::{ApiError, ErrorClass};

/// Where the estate installs: the scope root every permission is tested on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Scope {
    /// `organizations/<number>`
    Org(String),
    /// `folders/<number>`
    Folder(String),
}

impl Scope {
    /// The full resource name (`organizations/N` / `folders/N`).
    pub(crate) fn resource(&self) -> &str {
        match self {
            Scope::Org(r) | Scope::Folder(r) => r,
        }
    }

    /// The bare numeric id.
    fn id(&self) -> &str {
        self.resource().split('/').nth(1).unwrap_or("")
    }

    /// The `setIamPolicy` permission of this resource type — probed alongside
    /// the required permissions to learn whether self-granting is possible.
    fn set_iam_policy_permission(&self) -> &'static str {
        match self {
            Scope::Org(_) => "resourcemanager.organizations.setIamPolicy",
            Scope::Folder(_) => "resourcemanager.folders.setIamPolicy",
        }
    }

    /// The gcloud command an administrator would run to grant `role` here.
    fn grant_command(&self, role: &str, member: &str) -> String {
        match self {
            Scope::Org(_) => format!(
                "gcloud organizations add-iam-policy-binding {} --member={} --role={}",
                self.id(),
                member,
                role
            ),
            Scope::Folder(_) => format!(
                "gcloud resource-manager folders add-iam-policy-binding {} --member={} --role={}",
                self.id(),
                member,
                role
            ),
        }
    }

    /// The matching removal, printed with every self-grant so it can be undone.
    fn revoke_command(&self, role: &str, member: &str) -> String {
        match self {
            Scope::Org(_) => format!(
                "gcloud organizations remove-iam-policy-binding {} --member={} --role={}",
                self.id(),
                member,
                role
            ),
            Scope::Folder(_) => format!(
                "gcloud resource-manager folders remove-iam-policy-binding {} --member={} --role={}",
                self.id(),
                member,
                role
            ),
        }
    }
}

/// Classify the normalized parent. Bootstrap installs under an organization
/// or a folder; anything else is a config error, named now rather than as a
/// downstream API rejection.
pub(crate) fn detect_scope(normalized_parent: &str) -> Result<Scope, String> {
    let p = normalized_parent.trim();
    let numeric_tail_of = |prefix: &str| {
        p.strip_prefix(prefix)
            .filter(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
    };
    if numeric_tail_of("organizations/").is_some() {
        return Ok(Scope::Org(p.to_string()));
    }
    if numeric_tail_of("folders/").is_some() {
        return Ok(Scope::Folder(p.to_string()));
    }
    Err(format!(
        "bootstrap installs under an organization or a folder, but `customer_organization_id` \
         resolves to {:?} — expected `organizations/<number>`, `folders/<number>`, or a bare \
         organization number",
        p
    ))
}

/// The permission on the billing account: bootstrap links the infra project.
pub(crate) const BILLING_PERMISSION: &str = "billing.resourceAssociations.create";
/// The role an administrator grants to supply [`BILLING_PERMISSION`].
pub(crate) const BILLING_ROLE: &str = "roles/billing.user";

/// The permissions bootstrap and the estate's first apply need on the scope
/// root, each paired with the role that supplies it (the self-grant target).
pub(crate) fn required_permissions(wants_folder: bool) -> Vec<(&'static str, &'static str)> {
    let mut perms = Vec::new();
    if wants_folder {
        perms.push(("resourcemanager.folders.create", "roles/resourcemanager.folderAdmin"));
    }
    perms.push(("resourcemanager.projects.create", "roles/resourcemanager.projectCreator"));
    // Not used by bootstrap itself, but the estate's org policies land here on
    // the first apply — finding out now beats finding out mid-apply.
    perms.push(("orgpolicy.policies.create", "roles/orgpolicy.policyAdmin"));
    perms
}

/// What the pre-flight decided. Pure, so the table is pinned by tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Everything required is granted.
    Proceed,
    /// Missing scope-root permissions, and the caller holds `setIamPolicy`
    /// there: grant these roles to the caller, re-test, then proceed.
    SelfGrant(Vec<String>),
    /// Cannot be fixed from here: print these commands for an administrator
    /// and stop before creating anything.
    Stop(Vec<String>),
}

/// The decision table.
///
/// `missing` are (permission, role) pairs absent on the scope root;
/// `billing_missing` is [`BILLING_PERMISSION`] absent on the billing account
/// (never self-granted in v1); `can_set_policy` is the scope's
/// `setIamPolicy` probe; `principal` is the caller (None = unknown, which
/// makes self-granting impossible and puts a placeholder in the commands).
pub(crate) fn decide(
    missing: &[(String, String)],
    billing_missing: bool,
    can_set_policy: bool,
    scope: &Scope,
    billing_account: &str,
    principal: Option<&str>,
) -> Decision {
    if missing.is_empty() && !billing_missing {
        return Decision::Proceed;
    }

    let member = principal.map(|p| format!("user:{}", p)).unwrap_or_else(|| "user:<YOUR_ADMIN_EMAIL>".to_string());

    // A missing billing permission cannot be self-granted, and an unknown
    // principal cannot be written into a binding: both mean stop, listing
    // every grant an administrator must make.
    if billing_missing || !can_set_policy || principal.is_none() {
        let mut commands = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for (_, role) in missing {
            if seen.insert(role.clone()) {
                commands.push(scope.grant_command(role, &member));
            }
        }
        if billing_missing {
            commands.push(format!(
                "gcloud billing accounts add-iam-policy-binding {} --member={} --role={}",
                billing_account, member, BILLING_ROLE
            ));
        }
        return Decision::Stop(commands);
    }

    let mut roles = Vec::new();
    for (_, role) in missing {
        if !roles.contains(role) {
            roles.push(role.clone());
        }
    }
    Decision::SelfGrant(roles)
}

/// A required permission paired with the role that supplies it.
pub(crate) type PermRole = (String, String);

/// On a folder scope, a permission whose supplying role Google allows only at
/// the organization level cannot be self-granted there (the folder grant is
/// INVALID_ARGUMENT — live-verified 2026-09-02) and must not block a
/// folder-scoped install either: it is split out as advisory — named in the
/// output, never silently waved through. At org scope nothing is advisory.
pub(crate) fn split_folder_advisory(
    scope: &Scope,
    missing: Vec<PermRole>,
) -> (Vec<PermRole>, Vec<PermRole>) {
    match scope {
        Scope::Org(_) => (missing, Vec::new()),
        Scope::Folder(_) => missing
            .into_iter()
            .partition(|(_, role)| role != "roles/orgpolicy.policyAdmin"),
    }
}

/// Is this scope root visible to the caller at all, and — when the estate also
/// binds a directory customer id — is it the organisation that customer owns?
///
/// `testIamPermissions` on a resource the caller cannot see returns the EMPTY
/// set, which is byte-identical to "you legitimately hold none of these". So a
/// wrong organisation id used to be reported as missing permissions, with
/// `add-iam-policy-binding` advice that could never work for anyone. Asking
/// first turns that into the truth: the organisation is not visible, or it is
/// not the customer's.
///
/// A folder scope is left alone: `organizations:search` does not list folders,
/// and the folder's own permission test is the check that matters there.
pub(crate) async fn resolve_organization(
    client: &reqwest::Client,
    token: &str,
    scope: &Scope,
    customer_id: Option<&str>,
    principal: Option<&str>,
) -> Result<(), String> {
    let Scope::Org(resource) = scope else { return Ok(()) };
    let id = resource.trim_start_matches("organizations/");
    let orgs = crate::gcp::resourcemanager::search_organizations(client, token)
        .await
        .map_err(|e| format!("could not list the organizations visible to these credentials: {}", e))?;
    let _ = id;
    match organization_verdict(resource, &orgs, customer_id, principal.unwrap_or("these credentials")) {
        Ok(line) => {
            println!("  ok       {}", line);
            Ok(())
        }
        Err(why) => Err(why),
    }
}

/// The verdict on one scope root, given what the search returned. Pure, so the
/// two failures it exists for are pinned by tests rather than by a live tenant.
fn organization_verdict(
    resource: &str,
    orgs: &[serde_json::Value],
    customer_id: Option<&str>,
    who: &str,
) -> Result<String, String> {
    let named = |o: &serde_json::Value| -> String {
        let name = o.get("name").and_then(|v| v.as_str()).unwrap_or("organizations/?").to_string();
        match o.get("displayName").and_then(|v| v.as_str()) {
            Some(d) => format!("{} ({})", name, d),
            None => name,
        }
    };
    let Some(found) = orgs.iter().find(|o| o.get("name").and_then(|v| v.as_str()) == Some(resource)) else {
        let visible = if orgs.is_empty() {
            "none are visible".to_string()
        } else {
            format!("visible: {}", orgs.iter().map(named).collect::<Vec<_>>().join(", "))
        };
        return Err(format!(
            "{} is not visible to {} — check `customer_organization_id` in the estate ({})",
            resource, who, visible
        ));
    };
    // The estate binds BOTH ids. One search resolves the directory customer to
    // its organisation, so a mismatch is caught here rather than on apply.
    if let Some(cid) = customer_id.map(str::trim).filter(|c| !c.is_empty()) {
        if let Some(owner) = found.get("directoryCustomerId").and_then(|v| v.as_str()).filter(|o| *o != cid) {
            return Err(format!(
                "the estate binds `customer_id = \"{}\"` and `customer_organization_id = \"{}\"`, but {} belongs to directory customer {} — one of the two is wrong",
                cid,
                resource.trim_start_matches("organizations/"),
                resource,
                owner
            ));
        }
    }
    Ok(format!("{} is visible to {}", named(found), who))
}

/// Run the pre-flight against the live IAM: test, decide, and — on a live run
/// with `setIamPolicy` in hand — self-grant, audibly, then re-test until IAM
/// propagation catches up. Returns only when bootstrap may create things;
/// every other outcome is an `Err` and nothing has been created.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    client: &reqwest::Client,
    token: &str,
    normalized_parent: &str,
    wants_folder: bool,
    billing_account: &str,
    principal: Option<&str>,
    customer_id: Option<&str>,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let scope = detect_scope(normalized_parent)?;
    let resource = scope.resource().to_string();

    println!("--- Pre-flight: permissions of the caller ---");
    // does this organisation exist, and is it the customer's — the question that
    // has to come before any permission verdict, because an invisible resource
    // answers a permission test with silence
    resolve_organization(client, token, &scope, customer_id, principal).await?;
    if let Scope::Folder(_) = scope {
        println!(
            "folder-scoped install ({}): org-root operations are out of scope and were not checked",
            resource
        );
    }

    // One testIamPermissions call on the scope root: the required permissions
    // plus the setIamPolicy probe.
    let required = required_permissions(wants_folder);
    let mut asked: Vec<&str> = required.iter().map(|(p, _)| *p).collect();
    asked.push(scope.set_iam_policy_permission());
    let granted = crate::gcp::resourcemanager::test_permissions(client, token, &resource, &asked)
        .await
        .map_err(|e| preflight_probe_error(&resource, e))?;

    let mut missing: Vec<(String, String)> = Vec::new();
    for (perm, role) in &required {
        if granted.iter().any(|g| g == perm) {
            println!("  ok       {} on {}", perm, resource);
        } else {
            println!("  MISSING  {} on {} ({})", perm, resource, role);
            missing.push((perm.to_string(), role.to_string()));
        }
    }
    // roles/orgpolicy.policyAdmin exists only at the organization level, so a
    // folder-scoped install can neither self-grant it nor be blocked by it.
    let (missing, advisory) = split_folder_advisory(&scope, missing);
    for (perm, role) in &advisory {
        println!(
            "  note: {} stays missing — {} is an organization-level role; the estate's \
             folder-level org policies will need an organization-level grant before their \
             first apply. Continuing.",
            perm, role
        );
    }

    let can_set_policy = granted.iter().any(|g| g == scope.set_iam_policy_permission());

    // And one on the billing account.
    let billing_granted = billing_probe(client, token, billing_account).await?;

    match decide(&missing, !billing_granted, can_set_policy, &scope, billing_account, principal) {
        Decision::Proceed => {
            println!("pre-flight: OK");
            Ok(())
        }
        Decision::SelfGrant(roles) => {
            let principal = principal.expect("decide() returns SelfGrant only with a principal");
            if dry_run {
                for role in &roles {
                    println!("  would self-grant {} to user:{} on {}", role, principal, resource);
                }
                return Err(format!(
                    "pre-flight: {} permission(s) missing — a live run would self-grant the roles above \
                     (the caller holds {})",
                    missing.len(),
                    scope.set_iam_policy_permission()
                )
                .into());
            }
            self_grant(client, token, &scope, &roles, principal).await?;
            retest(client, token, &resource, &missing).await?;
            println!("pre-flight: OK (after self-grant)");
            Ok(())
        }
        Decision::Stop(commands) => {
            eprintln!("\nThe caller lacks required permissions that cannot be self-granted from here.");
            eprintln!("An administrator can grant them with:\n");
            for c in &commands {
                eprintln!("  {}", c);
            }
            eprintln!();
            Err("bootstrap stopped before creating anything: required permissions are missing".into())
        }
    }
}

/// A pre-flight probe that itself failed — the caller cannot even ask. A
/// quota-class 403 gets its own explanation instead of reading as a denial.
/// Whether the caller holds [`BILLING_PERMISSION`] on the billing account,
/// printed as a pre-flight line.
async fn billing_probe(
    client: &reqwest::Client,
    token: &str,
    billing_account: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let granted = crate::gcp::billing::test_billing_permissions(client, token, billing_account, &[BILLING_PERMISSION])
        .await
        .map_err(|e| preflight_probe_error(&format!("billingAccounts/{}", billing_account), e))?
        .iter()
        .any(|g| g == BILLING_PERMISSION);
    if granted {
        println!("  ok       {} on billingAccounts/{}", BILLING_PERMISSION, billing_account);
    } else {
        println!(
            "  MISSING  {} on billingAccounts/{} ({})",
            BILLING_PERMISSION, billing_account, BILLING_ROLE
        );
    }
    Ok(granted)
}

/// The billing half of the pre-flight, on its own, for greenfield: the scope
/// root the rest is tested on does not exist until the parentless project
/// creation materializes it, and that project is a real resource. What can be
/// tested before it — the billing account the project will be linked to — is
/// tested before it. Read-only; bootstrap cannot self-grant on a billing account.
pub(crate) async fn billing(
    client: &reqwest::Client,
    token: &str,
    billing_account: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("--- Pre-flight (greenfield, before anything is created): the billing account ---");
    if billing_probe(client, token, billing_account).await? {
        return Ok(());
    }
    Err(format!(
        "pre-flight: {} is missing on billingAccounts/{} — a billing account administrator grants {} \
         to the caller, then re-run; nothing was created",
        BILLING_PERMISSION, billing_account, BILLING_ROLE
    )
    .into())
}

fn preflight_probe_error(resource: &str, e: ApiError) -> String {
    match e.class() {
        ErrorClass::QuotaProject => format!(
            "pre-flight could not test permissions on {}: {} — this is the quota project, not \
             your permissions; run `gcloud auth application-default set-quota-project <project>`",
            resource, e
        ),
        _ => format!("pre-flight could not test permissions on {}: {}", resource, e),
    }
}

/// Grant each role to the caller on the scope root — read-modify-write with
/// the policy's etag, retrying a concurrent change — and say exactly what was
/// granted and how to undo it.
async fn self_grant(
    client: &reqwest::Client,
    token: &str,
    scope: &Scope,
    roles: &[String],
    principal: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let resource = scope.resource();
    let member = format!("user:{}", principal);
    for role in roles {
        let mut attempts = 0;
        loop {
            attempts += 1;
            let mut policy = crate::gcp::resourcemanager::get_iam_policy(client, token, resource)
                .await
                .map_err(|e| format!("could not read the IAM policy of {}: {}", resource, e))?;
            if !crate::gcp::add_binding(&mut policy, role, &member) {
                println!("  {} already bound to {} on {}", role, member, resource);
                break;
            }
            match crate::gcp::resourcemanager::set_iam_policy(client, token, resource, &policy).await {
                Ok(()) => {
                    println!(
                        "  self-granted {} to {} on {} — remove with: {}",
                        role,
                        member,
                        resource,
                        scope.revoke_command(role, &member)
                    );
                    break;
                }
                // A concurrent IAM change invalidated the etag: re-read and retry.
                Err(e) if e.class() == ErrorClass::Conflict && attempts < 3 => continue,
                Err(e) => {
                    return Err(format!("could not grant {} on {}: {}", role, resource, e).into());
                }
            }
        }
    }
    Ok(())
}

/// After a self-grant, wait for IAM propagation: re-test the previously
/// missing permissions, bounded. Never proceed on hope.
async fn retest(
    client: &reqwest::Client,
    token: &str,
    resource: &str,
    missing: &[(String, String)],
) -> Result<(), Box<dyn std::error::Error>> {
    let perms: Vec<&str> = missing.iter().map(|(p, _)| p.as_str()).collect();
    for attempt in 1..=6u32 {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let granted = crate::gcp::resourcemanager::test_permissions(client, token, resource, &perms)
            .await
            .map_err(|e| preflight_probe_error(resource, e))?;
        let still_missing: Vec<&&str> = perms.iter().filter(|p| !granted.iter().any(|g| g == **p)).collect();
        if still_missing.is_empty() {
            return Ok(());
        }
        println!(
            "  waiting for IAM propagation ({}/6): still missing {}",
            attempt,
            still_missing.iter().map(|p| **p).collect::<Vec<_>>().join(", ")
        );
    }
    Err("the self-granted permissions did not become visible within 60s — \
         the grant was made (see above); re-run bootstrap once IAM has propagated"
        .into())
}

// Tests (pure layer only — no network).
#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> String {
        v.to_string()
    }

    // --- scope detection ---------------------------------------------------

    #[test]
    fn scopes_are_org_or_folder_and_nothing_else() {
        assert_eq!(detect_scope("organizations/123456789012"), Ok(Scope::Org(s("organizations/123456789012"))));
        assert_eq!(detect_scope("folders/424242"), Ok(Scope::Folder(s("folders/424242"))));
        assert!(detect_scope("projects/acme-iac-infra").is_err());
        assert!(detect_scope("folders/").is_err());
        assert!(detect_scope("folders/not-a-number").is_err());
        assert!(detect_scope("").is_err());
    }

    // --- required permission set -------------------------------------------

    #[test]
    fn folder_creation_is_only_required_when_the_estate_wants_a_folder() {
        let with = required_permissions(true);
        assert!(with.iter().any(|(p, _)| *p == "resourcemanager.folders.create"));
        let without = required_permissions(false);
        assert!(!without.iter().any(|(p, _)| *p == "resourcemanager.folders.create"));
        for set in [&with, &without] {
            assert!(set.iter().any(|(p, _)| *p == "resourcemanager.projects.create"));
            assert!(set.iter().any(|(p, _)| *p == "orgpolicy.policies.create"));
        }
    }

    // --- the decision table ------------------------------------------------

    fn org() -> Scope {
        Scope::Org(s("organizations/123456789012"))
    }

    #[test]
    fn all_granted_proceeds() {
        let d = decide(&[], false, false, &org(), "A12345-B67890-C12345", Some("admin@example.com"));
        assert_eq!(d, Decision::Proceed);
    }

    #[test]
    fn missing_with_set_iam_policy_self_grants_deduped_roles() {
        let missing = vec![
            (s("resourcemanager.folders.create"), s("roles/resourcemanager.folderAdmin")),
            (s("orgpolicy.policies.create"), s("roles/orgpolicy.policyAdmin")),
        ];
        let d = decide(&missing, false, true, &org(), "A12345-B67890-C12345", Some("admin@example.com"));
        assert_eq!(
            d,
            Decision::SelfGrant(vec![s("roles/resourcemanager.folderAdmin"), s("roles/orgpolicy.policyAdmin")])
        );
    }

    #[test]
    fn missing_without_set_iam_policy_stops_with_exact_commands() {
        let missing = vec![(s("resourcemanager.folders.create"), s("roles/resourcemanager.folderAdmin"))];
        let d = decide(&missing, false, false, &org(), "A12345-B67890-C12345", Some("admin@example.com"));
        let Decision::Stop(cmds) = d else { panic!("expected Stop, got {:?}", d) };
        assert_eq!(
            cmds,
            vec![s(
                "gcloud organizations add-iam-policy-binding 123456789012 \
                 --member=user:admin@example.com --role=roles/resourcemanager.folderAdmin"
            )
            .replace("  ", " ")]
        );
    }

    #[test]
    fn missing_billing_permission_always_stops_and_names_the_billing_grant() {
        // Even with setIamPolicy in hand: billing is never self-granted in v1.
        let d = decide(&[], true, true, &org(), "A12345-B67890-C12345", Some("admin@example.com"));
        let Decision::Stop(cmds) = d else { panic!("expected Stop, got {:?}", d) };
        assert_eq!(
            cmds,
            vec![s(
                "gcloud billing accounts add-iam-policy-binding A12345-B67890-C12345 \
                 --member=user:admin@example.com --role=roles/billing.user"
            )
            .replace("  ", " ")]
        );
    }

    #[test]
    fn unknown_principal_stops_with_a_placeholder_member() {
        let missing = vec![(s("resourcemanager.projects.create"), s("roles/resourcemanager.projectCreator"))];
        let d = decide(&missing, false, true, &org(), "A12345-B67890-C12345", None);
        let Decision::Stop(cmds) = d else { panic!("expected Stop, got {:?}", d) };
        assert!(cmds[0].contains("user:<YOUR_ADMIN_EMAIL>"), "{:?}", cmds);
    }

    #[test]
    fn folder_scope_prints_folder_commands() {
        let scope = Scope::Folder(s("folders/424242"));
        let missing = vec![(s("resourcemanager.projects.create"), s("roles/resourcemanager.projectCreator"))];
        let d = decide(&missing, false, false, &scope, "A12345-B67890-C12345", Some("admin@example.com"));
        let Decision::Stop(cmds) = d else { panic!("expected Stop, got {:?}", d) };
        assert!(cmds[0].starts_with("gcloud resource-manager folders add-iam-policy-binding 424242"), "{:?}", cmds);
    }

    #[test]
    fn folder_scope_makes_the_org_level_only_role_advisory() {
        let missing = vec![
            (s("resourcemanager.projects.create"), s("roles/resourcemanager.projectCreator")),
            (s("orgpolicy.policies.create"), s("roles/orgpolicy.policyAdmin")),
        ];
        let (blocking, advisory) =
            split_folder_advisory(&Scope::Folder(s("folders/424242")), missing.clone());
        assert_eq!(blocking, vec![missing[0].clone()]);
        assert_eq!(advisory, vec![missing[1].clone()]);

        // At org scope the role IS grantable — nothing becomes advisory.
        let (blocking, advisory) = split_folder_advisory(&org(), missing.clone());
        assert_eq!(blocking, missing);
        assert!(advisory.is_empty());
    }

    fn seen_org(name: &str, display: &str, customer: &str) -> serde_json::Value {
        serde_json::json!({"name": name, "displayName": display, "directoryCustomerId": customer})
    }

    #[test]
    fn an_organisation_the_caller_cannot_see_is_named_as_that_not_as_missing_permissions() {
        // testIamPermissions answers an invisible resource with the EMPTY set,
        // which reads exactly like "you hold none of these" — so the question
        // has to be asked before any permission verdict
        let visible = [seen_org("organizations/111", "acme.example", "C0acme")];
        let err = organization_verdict("organizations/999", &visible, None, "someone@example.com").unwrap_err();
        assert!(err.contains("organizations/999 is not visible to someone@example.com"), "{}", err);
        assert!(err.contains("check `customer_organization_id`"), "{}", err);
        assert!(err.contains("organizations/111 (acme.example)"), "it says what IS visible:\n{}", err);
        let err = organization_verdict("organizations/999", &[], None, "someone@example.com").unwrap_err();
        assert!(err.contains("none are visible"), "{}", err);
    }

    #[test]
    fn the_two_ids_the_estate_binds_are_cross_checked_against_each_other() {
        let visible = [seen_org("organizations/111", "acme.example", "C0acme")];
        // visible, but not this customer's
        let err = organization_verdict("organizations/111", &visible, Some("C0other"), "who").unwrap_err();
        assert!(err.contains("customer_id = \"C0other\""), "{}", err);
        assert!(err.contains("belongs to directory customer C0acme"), "{}", err);
        // matching, absent and blank all pass
        assert!(organization_verdict("organizations/111", &visible, Some("C0acme"), "who").is_ok());
        assert!(organization_verdict("organizations/111", &visible, None, "who").is_ok());
        assert!(organization_verdict("organizations/111", &visible, Some("  "), "who").is_ok());
        // an organisation the search reports without a directory customer id
        let bare = [serde_json::json!({"name": "organizations/111"})];
        assert!(organization_verdict("organizations/111", &bare, Some("C0acme"), "who").is_ok());
    }
}
