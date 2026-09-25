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

use satz_core::pack_graph::{Node, PackGraph};
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
    /// The display name of the folder directly under the organisation that holds the
    /// customer's and the teams' folders; `None` when they live at the organisation.
    pub workload_folder_name: Option<String>,
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
  // the roles this estate's resource types need — `satz update-prerequisites` adds the ones
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
          "storage.googleapis.com",
        ]
        google_storage_bucket {
          state {
            "import-id"                 = infra_bucket_name
            name                        = infra_bucket_name
            location                    = default_region
            force_destroy               = true
            public_access_prevention    = "enforced"
            uniform_bucket_level_access = true
            versioning { enabled = true }
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

// The folder, published to the HCL teams write beside the estate (hcl/interfaces/). Its
// number exists once the folder does, so the interface modules look it up.
export "infra_folder" = "${{google_folder.infra_folder.name}}" description "The folder that holds the infrastructure project, folders/<number>"
"#;

/// The param that names the workload folder; empty is the organisation.
pub(crate) const WORKLOAD_FOLDER_NAME: &str = "workload_folder_name";

/// The workload folder: where the customer's and the teams' folders live, published as the
/// core export `workload_folder` — the organisation (`organizations/<id>`, a static value,
/// nothing created) or one folder directly under it (`folders/<number>`, looked up). `init`
/// writes it from its flag, a re-run with the flag through [`with_workload_folder`]. The
/// folder's name is the param, so renaming it moves nothing here.
pub(crate) fn workload_folder_section(folder: bool) -> String {
    const HEAD: &str = "// ---- the workload folder: where the customer's and the teams' folders live ----------\n";
    const DESCRIPTION: &str = "Where the customer's and the teams' folders live: organizations/<id>, or folders/<number> for a folder directly under it";
    if folder {
        format!(
            r#"{HEAD}// One folder directly under the organisation, `workload_folder_name`. Google refuses a
// second folder of one name under one parent: a folder the customer already has is imported
// (`satz adopt <estate> --execute --import`), and until it is, the apply stops on it.
google_folder {{
  workload_folder {{
    display_name = workload_folder_name
  }}
}}

// Published to the teams' HCL, whose folders take it as their parent. The folder's number
// exists once the folder does, so the interface modules look it up.
export "workload_folder" = "${{{{google_folder.workload_folder.name}}}}" description "{DESCRIPTION}"
"#
        )
    } else {
        format!(
            r#"{HEAD}// The organisation itself. Published to the teams' HCL, whose folders take it as their parent.
export "workload_folder" = "organizations/{{customer_organization_id}}" description "{DESCRIPTION}"
"#
        )
    }
}

/// `src` with its workload folder in the form `folder` names: the section appended when the
/// estate publishes none, `src` unchanged when it already publishes that form. The other
/// form is refused, never rewritten: the teams' folders sit under the root, and turning a
/// folder into the organisation, or the reverse, moves every one of them.
pub(crate) fn with_workload_folder(src: &str, folder: bool) -> Result<String, String> {
    let export = src
        .lines()
        .enumerate()
        .find(|(_, l)| l.trim_start().starts_with("export \"workload_folder\""));
    let Some((n, line)) = export else {
        let mut out = src.to_string();
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
        out.push_str(&workload_folder_section(folder));
        return Ok(out);
    };
    let is_folder = line.contains("google_folder.workload_folder.");
    if is_folder == folder {
        return Ok(src.to_string());
    }
    let (now, edit) = if is_folder {
        (
            "the folder `google_folder.workload_folder`",
            "once the teams' folders have moved to the organisation, remove the folder and export \"organizations/{customer_organization_id}\"",
        )
    } else {
        (
            "the organisation",
            "declare `google_folder { workload_folder { display_name = workload_folder_name } }` and export \"${{google_folder.workload_folder.name}}\"",
        )
    };
    Err(format!(
        "{}: line {} publishes the workload folder as {}. Changing it moves every folder the teams created under it, so satz does not rewrite it — edit that section by hand ({}), then run this again",
        if folder { "a workload folder" } else { "the organisation as the workload folder" },
        n + 1,
        now,
        edit
    ))
}

/// Where an offered pack's line goes in the estate satz writes: the menu, or inside the
/// resource type map the graph's `block` names. A pack is used at the top level of the
/// estate, so the menu holds every line but those of the bare lists, which are the content
/// of their type's map.
///
/// The menu is one list in the graph's order, and the compile builds one param namespace
/// in file order: a pack that reads another pack's param has its line after that pack's,
/// which is the order of the `offers` entries.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place<'a> {
    Menu,
    Block(&'a str),
}

pub(crate) fn place(n: &Node) -> Place<'_> {
    match &n.block {
        Some(b) => Place::Block(b.as_str()),
        None => Place::Menu,
    }
}

/// A phase caption as comment lines, each at `pad`: the graph stores the text bare, one
/// line per line, and every line is written with `// `.
pub(crate) fn phase_comment(phase: &str, pad: &str) -> String {
    let mut out = String::new();
    for l in phase.lines().filter(|l| !l.trim().is_empty()) {
        out.push_str(pad);
        out.push_str("// ");
        out.push_str(l.trim());
        out.push('\n');
    }
    out
}

/// The commented menu, as the skeleton writes it: one line per pack the graph offers at the
/// top level, a phase comment above each group, and the whole thing inert until a line is
/// uncommented.
fn pack_menu(graph: &PackGraph) -> String {
    pack_group(
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
         // Whichever writes it, the compile reports a question THIS estate has answered true\n\
         // whose line is still commented, so the two never drift apart. A default in the map or\n\
         // in a pack is the library's proposal, not this estate's answer: `satz packs` lists it,\n\
         // and the compile says nothing until the answer is bound here.\n",
        graph,
        Place::Menu,
    )
}

/// One group of commented pack lines: the offered packs placed `at`, each phase printed
/// once above the lines it heads. A pack written inside a resource type map is written into
/// that map, not into a group.
fn pack_group(header: &str, graph: &PackGraph, at: Place) -> String {
    let mut out = String::from(header);
    for n in graph.lines().into_iter().filter(|n| place(n) == at) {
        if let Some(phase) = n.phase.as_deref().filter(|p| !p.trim().is_empty()) {
            out.push('\n');
            out.push_str(&phase_comment(phase, ""));
        }
        out.push_str(&pack_line(&n.path, n.gate.as_deref()));
        out.push('\n');
    }
    out
}

/// Write every offered pack written inside a resource type map into that map of `src`,
/// writing the map itself where `src` does not have it.
fn place_in_maps(mut src: String, graph: &PackGraph) -> String {
    for n in graph.lines() {
        if let Place::Block(at) = place(n) {
            src = insert_into_map(&src, at, &pack_line(&n.path, n.gate.as_deref()), n.phase.as_deref().unwrap_or(""))
                .unwrap_or_else(|| append_map(&src, at, &pack_line(&n.path, n.gate.as_deref()), n.phase.as_deref().unwrap_or("")));
        }
    }
    src
}

/// The whole menu of `graph` written into `src`, an estate that carries no pack line —
/// `init` or `interview --create` wrote it without a graph: the top-level lines after the
/// estate-core line, and the lines of the bare lists inside the map of their type. The
/// result is the file those commands write with the graph. `None` when `src` has no
/// estate-core line to place the menu after.
pub(crate) fn with_menu(src: &str, graph: &PackGraph) -> Option<String> {
    let core = "use \"presets/estate-core.satz\"";
    let mut at = 0usize;
    let mut found = false;
    for raw in src.split_inclusive('\n') {
        at += raw.len();
        if found {
            // past one blank line after it, where the skeleton leaves one
            if raw.trim().is_empty() {
                break;
            }
            at -= raw.len();
            break;
        }
        found = raw.trim().trim_start_matches("// ") == core;
    }
    if !found {
        return None;
    }
    let mut out = format!("{}{}\n{}", &src[..at], pack_menu(graph), &src[at..]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(place_in_maps(out, graph))
}

/// Insert `line` (and its `//` phase comment, when given) as the first content of the
/// resource type map `at` names, at the top level of `src`. The indentation is two spaces,
/// so the result is already in the canonical layout.
///
/// `None` when the estate has no such map — [`append_map`] writes it whole.
///
/// Both writers call this — the skeleton as it composes a new file, `merge-presets` on an
/// estate that has no line for the pack — so the line lands in one place, not two.
pub(crate) fn insert_into_map(src: &str, at: &str, line: &str, phase: &str) -> Option<String> {
    let mut open_at: Option<(usize, usize)> = None; // (byte after the opening line, indent)
    let mut i = 0usize;
    let mut depth = 0usize;
    for raw in src.split_inclusive('\n') {
        let start = i;
        i += raw.len();
        let trimmed = raw.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        // the map this line opens, at the top level: `<type> {`
        if depth == 0 && trimmed.strip_suffix('{').map(str::trim_end).is_some_and(|n| n == at) {
            let indent = raw.len() - raw.trim_start().len();
            open_at = Some((start + raw.len(), indent));
            break;
        }
        depth += trimmed.matches('{').count();
        depth = depth.saturating_sub(trimmed.matches('}').count());
    }
    let (mut at_byte, indent) = open_at?;
    // past the block's own attributes, comments and any pack line already there,
    // and stop at its first child block: a line lands where the skeleton wrote
    // it, and a second line for the same block lands after the first, so the
    // graph's order is the file's order
    for raw in src[at_byte..].split_inclusive('\n') {
        let t = raw.trim();
        let is_own = t.is_empty()
            || t.starts_with("//")
            || (!t.ends_with('{') && !t.starts_with('}') && !t.starts_with(']'));
        if !is_own {
            break;
        }
        at_byte += raw.len();
    }
    let pad = " ".repeat(indent + 2);
    let mut insert = phase_comment(phase, &pad);
    insert.push_str(&pad);
    insert.push_str(line);
    insert.push('\n');
    let mut out = String::with_capacity(src.len() + insert.len());
    out.push_str(&src[..at_byte]);
    out.push_str(&insert);
    out.push_str(&src[at_byte..]);
    Some(out)
}

/// `src` with the resource type map `at` names appended, holding `line` — for an estate
/// that has no such map. A map is content the estate may always carry: an empty one emits
/// nothing, and the line inside it is commented until the pack is switched on.
pub(crate) fn append_map(src: &str, at: &str, line: &str, phase: &str) -> String {
    let mut out = src.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&phase_comment(phase, ""));
    out.push_str(&format!("{} {{\n  {}\n}}\n", at, line));
    out
}

/// One commented line, exactly as both writers must write it.
pub(crate) fn pack_line(path: &str, gate: Option<&str>) -> String {
    match gate {
        None => format!("// use \"{}\"", path),
        Some(gate) => format!("// use \"{}\" when {}", path, gate),
    }
}

/// Switch `use "presets/estate-core.satz"` on in an estate `init` wrote, where it is
/// commented out. A splice through `write_edited_satz`, so the author's layout stays.
/// `false` when the line is already active, or the estate never mentions the pack —
/// the interview then says what it finds.
pub(crate) fn use_estate_core(estate: &Path) -> Result<bool, Box<dyn std::error::Error>> {
    const COMMENTED: &str = "// use \"presets/estate-core.satz\"";
    let before = crate::fsx::read_to_string(estate)?;
    let Some(line) = before.lines().find(|l| l.trim() == COMMENTED) else {
        return Ok(false);
    };
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    let after = before.replacen(line, &format!("{}use \"presets/estate-core.satz\"", indent), 1);
    crate::fsx::write_edited_satz(estate, &before, &after)?;
    Ok(true)
}

/// The estate an INTERVIEW starts from: the day-0 scaffold, and every pack commented out.
///
/// The day-0 params and their questions come from `presets/estate-core.satz`, and that is
/// the only `use` the file starts with — the day-0 questions, all of them about the estate
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
///
/// The lines come from `graph`, the pack graph that arrived with the presets. Without one
/// the file carries no pack lines at all; `satz get-presets` then `satz merge-presets`
/// write them.
pub(crate) fn skeleton(stem: &str, graph: Option<&PackGraph>) -> String {
    let composed = format!(
        r#"// Written for an interview: a question is open until its param is bound below.
// `satz questions {stem}.satz --unanswered --format text --out -` lists what is still to decide;
// bootstrap and apply refuse until nothing is.

estate {estate}

params {{
}}

// The day-0 params and their questions. The only pack the file starts with: its
// questions, every one of them about the estate itself.
use "presets/estate-core.satz"

{menu}google_essential_contacts_contact {{
}}

{scaffold}"#,
        stem = stem,
        estate = estate_name(stem),
        scaffold = SCAFFOLD,
        menu = graph.map(|g| format!("{}\n", pack_menu(g))).unwrap_or_default(),
    );
    // the bare lists go into the map of their type, from the same graph
    // `merge-presets` reads, so such a line has one writer and not two
    match graph {
        Some(g) => place_in_maps(composed, g),
        None => composed,
    }
}

/// The skeleton with no pack line in it: what a graph's placements are read against.
pub(crate) fn bare_skeleton() -> String {
    skeleton("x", None)
}

/// The estate `satz init` writes, with the pack lines of `graph` — none without one, as
/// [`skeleton`] does.
pub fn generate_template(args: &TemplateArgs, graph: Option<&PackGraph>, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let workload_folder = format!(
        "  workload_folder_name = {:?}\n",
        args.workload_folder_name.as_deref().unwrap_or("")
    );
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
  // The compliance frameworks this customer is held to, as catalog ids from
  // `presets/catalogs/`. Not what this estate claims — that comes from its packs.
  compliance_frameworks    = ["cis-gcp-5.0"]
  // The folder the audit archive's project is created in, read by
  // `presets/monitoring/organization-audit-logsink.satz`. Bound here because init writes
  // the folder: unbound, that pack creates its project under the organisation.
  logsink_project_folder = "google_folder.infra_folder.name"
  // Where the customer's and the teams' folders live: "" is the organisation, a name one
  // folder directly under it. The section at the end of this file publishes it as
  // `workload_folder`; the compile refuses a name and a section that disagree.
{workload_folder}}}

// The day-0 params and their questions: init binds what it derived above, and this pack
// declares the rest, each with its question. It stays commented until the library is
// fetched, so the estate compiles with nothing downloaded — `satz init --interview`
// fetches it, switches it on, and asks what init could not derive.
// use "presets/estate-core.satz"

{menu}google_essential_contacts_contact {{
}}

{scaffold}
{workload_folder_section}"#,
        scaffold = SCAFFOLD,
        workload_folder = workload_folder,
        workload_folder_section = workload_folder_section(args.workload_folder_name.is_some()),
        menu = graph.map(|g| format!("{}\n", pack_menu(g))).unwrap_or_default(),
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
    // the bare lists into the map of their type, from the same graph — so `init` and
    // `interview --create` produce one shape and a pack is adoptable from
    // either door
    let content = match graph {
        Some(g) => place_in_maps(content, g),
        None => content,
    };
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

    /// The pack graph the repository ships beside its presets.
    pub(crate) fn shipped() -> PackGraph {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/pack-graph.json")).unwrap();
        serde_json::from_str(&text).unwrap()
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
            workload_folder_name: None,
        }
    }

    /// `bootstrap` creates the state bucket with versioning on (`src/gcp/storage.rs`, whose header
    /// calls it non-negotiable for a Terraform state bucket), and the scaffold's two lifecycle rules
    /// count versions. A skeleton that does not DECLARE versioning therefore plans to switch it off
    /// on the bucket holding the state, beside `force_destroy = true`.
    #[test]
    fn the_scaffold_declares_versioning_on_the_state_bucket() {
        let sk = skeleton("acme", None);
        let after = sk.split_once("google_storage_bucket {").expect("the scaffold declares the state bucket").1;
        let bucket = &after[..after.find("google_service_account").unwrap_or(after.len())];
        assert!(
            bucket.contains("versioning { enabled = true }"),
            "the state bucket must declare versioning: bootstrap creates it with versioning on, so a skeleton without it plans to turn it off\n{}",
            bucket
        );
        for rule in ["num_newer_versions", "days_since_noncurrent_time"] {
            assert!(bucket.contains(rule), "the version-counting lifecycle rule {} is still declared", rule);
        }
    }

    /// The templates are canonical at the source: what `init` and the interview write
    /// is what the formatter would write, so the source reads as the file does. A
    /// failure here is fixed in the template, never in the test.
    #[test]
    fn the_skeleton_is_in_the_canonical_layout() {
        let sk = skeleton("acme", Some(&shipped()));
        assert_eq!(satz_core::fmt::format(&sk).unwrap(), sk, "template::skeleton is not formatted");
    }

    #[test]
    fn the_init_estate_is_in_the_canonical_layout() {
        let dir = scratch("canon");
        let path = dir.join("C0example.satz");
        generate_template(&args("first.admin", "example.com"), Some(&shipped()), &path).unwrap();
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
    fn a_placed_line_lands_in_its_map_and_a_second_one_after_the_first() {
        // the map's own content first, then the pack lines in graph order
        let src = "google_essential_contacts_contact {\n  all {\n    email = \"x\"\n  }\n}\n";
        let one = insert_into_map(src, "google_essential_contacts_contact", "// use \"a.satz\" when a", "first").unwrap();
        let two = insert_into_map(&one, "google_essential_contacts_contact", "// use \"b.satz\" when b", "").unwrap();
        let at = |s: &str, needle: &str| two.lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{} not in {}", needle, s));
        assert!(at(&two, "a.satz") < at(&two, "b.satz"), "graph order is file order:\n{}", two);
        assert!(at(&two, "b.satz") < at(&two, "all {"), "before the map's own content:\n{}", two);
        assert!(two.contains("  // first\n"), "the phase comment rides along, at the line's indent:\n{}", two);
        // a map the estate does not have is written whole, at the end
        assert_eq!(insert_into_map(src, "google_org_policy_policy", "// use \"c.satz\"", ""), None);
        let appended = append_map(src, "google_org_policy_policy", "// use \"c.satz\"", "why");
        assert!(appended.starts_with(src), "{}", appended);
        assert!(appended.ends_with("// why\ngoogle_org_policy_policy {\n  // use \"c.satz\"\n}\n"), "{}", appended);
        // a map nested inside a node is not the top-level one the graph names
        let nested = "google_folder {\n  infra_folder {\n    google_essential_contacts_contact {\n    }\n  }\n}\n";
        assert_eq!(insert_into_map(nested, "google_essential_contacts_contact", "// use \"c.satz\"", ""), None);
    }

    #[test]
    fn the_skeleton_carries_one_use_line_per_choice_the_map_declares() {
        // Two places for one list — the map declares the choices, the skeleton carries their
        // `use … when` lines. This is what keeps them equal.
        let map = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/estate-map.satz")).unwrap();
        let map = satz_core::satz::parse(&map).unwrap();
        let sk = skeleton("x", Some(&shipped()));
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
            sk.contains("\n// use \"presets/monitoring/organization-audit-logsink.satz\" when use_audit_logsink\n"),
            "every pack line satz writes stands at the top level, commented"
        );
        // the one exception: a bare list is the content of the map of its type
        assert!(
            sk.contains("  // use \"presets/essential-contacts-organization.satz\" when use_essential_contacts\n}\n"),
            "the contacts pack's line is written inside the map of its type:\n{}",
            sk
        );
        for line in sk.lines().filter(|l| l.trim_start().starts_with("// use \"presets/")) {
            let indented = line.starts_with(' ');
            assert_eq!(
                indented,
                line.contains("essential-contacts-organization"),
                "a pack line stands at the top level unless it is a bare list: {}",
                line
            );
        }
    }

    #[test]
    fn every_map_choice_has_a_phase_and_the_menu_is_inert() {
        // The graph is what `merge-presets` inserts from, so a pack the library gains without
        // an entry would be a pack nobody can adopt — the map would ask for it and no line would
        // ever be written. This is that gate.
        let map = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/estate-map.satz")).unwrap();
        let map = satz_core::satz::parse(&map).unwrap();
        // No exceptions, a pack whose line lives inside a block included.
        let graph = shipped();
        let gated: Vec<&str> = graph.lines().iter().filter_map(|n| n.gate.as_deref()).collect();
        for (name, _, _) in &map.params {
            assert!(gated.contains(&name.as_str()), "the map declares `{}` and no line the graph offers is gated on it", name);
        }
        // and the menu enforces nothing until a line is uncommented
        let sk = skeleton("x", Some(&shipped()));
        for line in use_lines(&sk).filter(|l| !l.contains("estate-core")) {
            assert!(line.starts_with("// use "), "a pack line in a fresh skeleton must be commented: {}", line);
        }
        // the day-0 pack is the exception: it is not in the menu at all
        assert!(sk.contains("\nuse \"presets/estate-core.satz\"\n"), "estate-core is the one pack a day-0 file uses");
    }

    /// Whether any `use` sits in these items, at any depth.
    fn uses_a_pack(items: &[satz_core::satz::Entry]) -> bool {
        use satz_core::satz::Entry;
        items.iter().any(|e| match e {
            Entry::Use { .. } => true,
            Entry::Map { body, .. } => uses_a_pack(body),
            Entry::Attr { .. } => false,
        })
    }

    /// A param is known from the line that declares it on — the compile builds one namespace
    /// in file order — so a pack whose defaults or resources read another pack's param has to
    /// come AFTER that pack's line. Written before it, the file compiles while the line is
    /// commented and stops with `unknown param` the moment its question is answered yes.
    /// What a pack reads is derived from the packs (`doc_packs::needs`), so a new pack with
    /// that shape fails here, not in an interview.
    #[test]
    fn a_pack_line_follows_the_lines_of_the_packs_whose_params_it_reads() {
        let presets = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        let library = crate::doc_packs::packs(&presets).unwrap();
        let file = |path: &str| {
            library
                .iter()
                .find(|(rel, _, _)| format!("presets/{}", rel.display()) == path)
                .map(|(_, f, _)| f)
                .unwrap_or_else(|| panic!("the graph offers {}, which is not in the library", path))
        };
        let declares = |path: &str, param: &str| file(path).params.iter().any(|(n, _, _)| n == param);
        let dir = scratch("order");
        let init = dir.join("C0example.satz");
        generate_template(&args("first.admin", "example.com"), Some(&shipped()), &init).unwrap();
        let graph = shipped();
        let written = [("the interview skeleton", skeleton("x", Some(&graph))), ("the init estate", std::fs::read_to_string(&init).unwrap())];
        let mut early = Vec::new();
        for (writer, text) in &written {
            let line_of = |path: &str, gate: Option<&str>| {
                text.lines()
                    .position(|l| l.trim() == pack_line(path, gate))
                    .unwrap_or_else(|| panic!("{} has no line for {}", writer, path))
            };
            for n in graph.lines() {
                let (path, gate) = (n.path.as_str(), n.gate.as_deref());
                // `needs` reads the pack's own file; a pack that used another would bring
                // that one's reads too, and this test would not see them
                assert!(!uses_a_pack(&file(path).items), "{} uses another pack; teach this test to follow it", path);
                let mut reads = crate::doc_packs::needs(file(path));
                // a contribution is merged into its param before the walk, so where the
                // contributing line stands decides nothing — only a READ needs the order
                for contributed in crate::doc_packs::contributed(file(path)) {
                    reads.remove(&contributed);
                }
                if let Some(gate) = gate {
                    reads.insert(gate.to_string());
                }
                for param in &reads {
                    // the day-0 params come first in both files: estate-core, or init's own params
                    if declares("presets/estate-core.satz", param) {
                        continue;
                    }
                    let providers: Vec<(&str, Option<&str>)> = graph
                        .lines()
                        .into_iter()
                        .filter(|p| p.path != path && declares(&p.path, param))
                        .map(|p| (p.path.as_str(), p.gate.as_deref()))
                        .collect();
                    // no pack declares it: the estate binds it, wherever the line is
                    if providers.is_empty() {
                        continue;
                    }
                    if !providers.iter().any(|(p, g)| line_of(p, *g) < line_of(path, gate)) {
                        let by: Vec<&str> = providers.iter().map(|(p, _)| *p).collect();
                        early.push(format!("{}: `{}` reads `{}` ({}), and its line comes before theirs", writer, path, param, by.join(", ")));
                    }
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            early.is_empty(),
            "a pack line before the line of a pack whose param it reads — answering it yes stops the compile with `unknown param`. The order of the lines is the order of the `offers` entries, so the entry has to move:\n  {}",
            early.join("\n  ")
        );
    }

    #[test]
    fn members_are_composed_from_params_not_written_verbatim() {
        // Changing `first_admin` or `customer_domain` must update every reference;
        // the old YAML template once wrote the full address and silently diverged.
        let dir = scratch("params");
        let path = dir.join("out.satz");
        generate_template(&args("first.admin", "example.com"), Some(&shipped()), &path).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains(r#"member       = ["user:{first_admin}@{customer_domain}"]"#), "{out}");
        assert!(!out.contains("first.admin@example.com"), "{out}");
        assert!(out.contains(r#"first_admin              = "first.admin""#), "{out}");
        assert!(out.contains(r#"customer_domain          = "example.com""#), "{out}");
        assert!(out.contains("estate c0example"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The section is written once, in the form asked for; the other form is refused,
    /// because the teams' folders sit under the root.
    #[test]
    fn the_workload_folder_section_follows_the_answer_and_is_never_flipped() {
        let sk = skeleton("x", None);
        assert!(!sk.contains("export \"workload_folder\""), "the skeleton publishes no root before it is answered");
        for folder in [false, true] {
            let one = with_workload_folder(&sk, folder).unwrap();
            assert!(one.ends_with(&workload_folder_section(folder)), "{}", one);
            assert_eq!(satz_core::fmt::format(&one).unwrap(), one, "the section is in the canonical layout");
            assert_eq!(with_workload_folder(&one, folder).unwrap(), one, "a second answer of the same form writes nothing");
            let err = with_workload_folder(&one, !folder).unwrap_err();
            assert!(err.contains("satz does not rewrite it"), "{}", err);
            assert!(err.starts_with(if folder { "the organisation" } else { "a workload folder" }), "{}", err);
            assert!(err.contains(&format!("line {}", one.lines().position(|l| l.starts_with("export \"workload_folder\"")).unwrap() + 1)), "{}", err);
        }
        assert!(
            workload_folder_section(true).contains("display_name = workload_folder_name"),
            "the folder reads its name from the param, so a rename moves nothing here"
        );
    }

    #[test]
    fn estate_names_are_identifiers() {
        assert_eq!(estate_name("C0example"), "c0example");
        assert_eq!(estate_name("0abc-x"), "_0abc_x");
    }
}
