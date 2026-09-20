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

/// Where an offered pack's line goes in the estate satz writes: the menu above the
/// scaffold, the group after it, or inside a block of it. The graph's `block` and
/// `after_scaffold` say which.
///
/// A line after the scaffold is a top-level pack that reads a param a pack INSIDE the
/// scaffold declares: the compile builds one namespace in file order, so a param is known
/// from the line that declares it on, and a line above the folder that reads the logsink's
/// params stops the compile with `unknown param` the moment it is uncommented.
/// `merge-presets` treats it as top level: it appends at the end of the file, which is
/// after the scaffold too.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place<'a> {
    Menu,
    AfterScaffold,
    Block(&'a str),
}

pub(crate) fn place(n: &Node) -> Place<'_> {
    match (&n.block, n.after_scaffold) {
        (Some(b), _) => Place::Block(b.as_str()),
        (None, true) => Place::AfterScaffold,
        (None, false) => Place::Menu,
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

/// The packs the skeleton writes after the scaffold, commented like the menu and under the
/// same phases.
fn packs_after_scaffold(graph: &PackGraph) -> String {
    pack_group(
        "\n// ---- the packs that read a param of a pack in the folder above ------------------\n\
         //\n\
         // A param is known from the line that declares it on, so a pack whose defaults name\n\
         // another pack's param comes after that pack's line. These read what the audit logsink\n\
         // or the central alerts declare — Sentinel's two log paths through Sentinel's own — and\n\
         // those two sit inside the infrastructure folder above. Uncomment a line, or answer its\n\
         // question yes, as with the list at the top.\n",
        graph,
        Place::AfterScaffold,
    )
}

/// One group of commented pack lines: the offered packs placed `at`, each phase printed
/// once above the lines it heads. A pack scoped to a block is written into that block, not
/// into a group.
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

/// Write every offered pack placed in a block into that block of `src`. `Err` names the
/// first block `src` lacks: the graph came with presets newer than this binary's scaffold.
fn place_in_blocks(mut src: String, graph: &PackGraph) -> Result<String, String> {
    for n in graph.lines() {
        if let Place::Block(at) = place(n) {
            src = insert_into_block(&src, at, &pack_line(&n.path, n.gate.as_deref()), n.phase.as_deref().unwrap_or(""))
                .ok_or_else(|| unknown_block(&n.path, at))?;
        }
    }
    Ok(src)
}

/// The whole menu of `graph` written into `src`, an estate that carries no pack line —
/// `init` or `interview --create` wrote it without a graph: the top-level lines after the
/// estate-core line, the group after the scaffold at the end, the block lines in their
/// blocks. The result is the file those commands write with the graph. `None` when `src`
/// has no estate-core line to place the menu after; an error when it lacks a block a line
/// belongs in.
pub(crate) fn with_menu(src: &str, graph: &PackGraph) -> Result<Option<String>, String> {
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
        return Ok(None);
    }
    let mut out = format!("{}{}\n{}", &src[..at], pack_menu(graph), &src[at..]);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&packs_after_scaffold(graph));
    place_in_blocks(out, graph).map(Some)
}

/// The refusal for a pack the graph places in a block the scaffold of this binary does not
/// have.
pub(crate) fn unknown_block(path: &str, block: &str) -> String {
    format!(
        "the pack graph places `{}` in `{}`, a block the estate this satz writes does not have — the presets are \
         newer than this binary: `satz self-update`, then run the command again",
        path, block
    )
}

/// Insert `line` (and its `//` phase comment, when given) as the first content of the block
/// `at` names — `google_folder.infra` is `infra { … }` inside `google_folder { … }`. The
/// indentation is the block's plus two, so the result is already in the canonical layout.
///
/// `None` when the estate has no such block: the caller reports that rather than writing the
/// line somewhere it does not belong. A `use` at the top level is valid anywhere, but one of
/// THESE packs is scoped by the block it sits in, so the wrong place is the wrong estate.
///
/// Both writers call this — the skeleton as it composes a new file, `merge-presets` on an
/// estate that has no line for the pack — so a nested pack lands in one place, not two.
pub(crate) fn insert_into_block(src: &str, at: &str, line: &str, phase: &str) -> Option<String> {
    let mut depth_wanted = 0usize;
    let mut open_at: Option<(usize, usize)> = None; // (byte after the opening line, indent)
    let segments: Vec<&str> = at.split('.').collect();
    let mut i = 0usize;
    let mut depth = 0usize;
    for raw in src.split_inclusive('\n') {
        let start = i;
        i += raw.len();
        let trimmed = raw.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        // a block this line opens: `<name> {` or `"<name>" {`
        if depth == depth_wanted && depth_wanted < segments.len() {
            let want = segments[depth_wanted];
            let opens = trimmed
                .strip_suffix('{')
                .map(str::trim_end)
                .is_some_and(|n| n == want || n.trim_matches('"') == want);
            if opens {
                depth_wanted += 1;
                depth += 1;
                if depth_wanted == segments.len() {
                    let indent = raw.len() - raw.trim_start().len();
                    open_at = Some((start + raw.len(), indent));
                    break;
                }
                continue;
            }
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

/// The block a single-segment placement names, written whole with its line inside — for an
/// estate that has no such block at all. Only a resource-type map is created this way
/// (`google_essential_contacts_contact`): it is content, and an empty one emits nothing. A
/// nested path names structure the estate owns (`google_folder.infra_folder`), and a missing
/// folder is reported, never invented.
pub(crate) fn block_stub(at: &str, line: &str, phase: &str) -> Option<String> {
    if at.contains('.') {
        return None;
    }
    let mut out = phase_comment(phase, "");
    out.push_str(&format!("{} {{\n  {}\n}}\n", at, line));
    Some(out)
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
///
/// The lines come from `graph`, the pack graph that arrived with the presets. Without one
/// the file carries no pack lines at all; `satz get-presets` then `satz merge-presets`
/// write them. `Err` when the graph places a pack in a block this binary's scaffold lacks.
pub(crate) fn skeleton(stem: &str, graph: Option<&PackGraph>) -> Result<String, String> {
    let composed = format!(
        r#"// Written for an interview: a question is open until its param is bound below.
// `satz questions {stem}.satz --unanswered --format text --out -` lists what is still to decide;
// bootstrap and apply refuse until nothing is.

estate {estate}

params {{
}}

// The day-0 params and their questions. The only pack the file starts with: sixteen
// questions, every one of them about the estate itself.
use "presets/estate-core.satz"

{menu}google_essential_contacts_contact {{
}}

{scaffold}{after}"#,
        stem = stem,
        estate = estate_name(stem),
        scaffold = SCAFFOLD,
        menu = graph.map(|g| format!("{}\n", pack_menu(g))).unwrap_or_default(),
        after = graph.map(packs_after_scaffold).unwrap_or_default(),
    );
    // the packs scoped to a block go into that block, from the same graph
    // `merge-presets` reads, so a nested pack has one writer and not two
    match graph {
        Some(g) => place_in_blocks(composed, g),
        None => Ok(composed),
    }
}

/// The skeleton with no pack line in it: the blocks this binary's scaffold has, which is
/// what a graph's placements are checked against.
pub(crate) fn bare_skeleton() -> String {
    skeleton("x", None).expect("a skeleton without a graph places no line, so it cannot fail")
}

/// The estate `satz init` writes, with the pack lines of `graph` — none without one, as
/// [`skeleton`] does.
pub fn generate_template(args: &TemplateArgs, graph: Option<&PackGraph>, output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
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

// The day-0 params and their questions: init binds what it derived above, and this pack
// declares the rest, each with its question. It stays commented until the library is
// fetched, so the estate compiles with nothing downloaded — `satz init --interview`
// fetches it, switches it on, and asks what init could not derive.
// use "presets/estate-core.satz"

{menu}google_essential_contacts_contact {{
}}

{scaffold}{after}"#,
        scaffold = SCAFFOLD,
        menu = graph.map(|g| format!("{}\n", pack_menu(g))).unwrap_or_default(),
        after = graph.map(packs_after_scaffold).unwrap_or_default(),
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
    // the packs scoped to a block, from the same graph — so `init` and
    // `interview --create` produce one shape and a pack is adoptable from
    // either door
    let content = match graph {
        Some(g) => place_in_blocks(content, g)?,
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
        }
    }

    /// The templates are canonical at the source: what `init` and the interview write
    /// is what the formatter would write, so the source reads as the file does. A
    /// failure here is fixed in the template, never in the test.
    #[test]
    fn the_skeleton_is_in_the_canonical_layout() {
        let sk = skeleton("acme", Some(&shipped())).unwrap();
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
    fn a_placed_line_lands_in_its_block_and_a_second_one_after_the_first() {
        // the block's own attributes first, then the pack lines in graph order,
        // then the block's children — where the skeleton used to write them by hand
        let src = "google_folder {\n  infra_folder {\n    display_name = infra_folder_name\n    google_project {\n      infra {\n      }\n    }\n  }\n}\n";
        let one = insert_into_block(src, "google_folder.infra_folder", "// use \"a.satz\" when a", "first").unwrap();
        let two = insert_into_block(&one, "google_folder.infra_folder", "// use \"b.satz\" when b", "").unwrap();
        let at = |s: &str, needle: &str| two.lines().position(|l| l.contains(needle)).unwrap_or_else(|| panic!("{} not in {}", needle, s));
        assert!(at(&two, "display_name") < at(&two, "a.satz"), "{}", two);
        assert!(at(&two, "a.satz") < at(&two, "b.satz"), "graph order is file order:\n{}", two);
        assert!(at(&two, "b.satz") < at(&two, "google_project"), "before the block's children:\n{}", two);
        assert!(two.contains("    // first\n"), "the phase comment rides along, at the line's indent:\n{}", two);
        // a block the estate does not have
        assert_eq!(insert_into_block(src, "google_essential_contacts_contact", "// use \"c.satz\"", ""), None);
        // …which a resource-type map answers by being written whole, and a folder does not
        assert!(block_stub("google_essential_contacts_contact", "// use \"c.satz\"", "why")
            .is_some_and(|s| s.contains("google_essential_contacts_contact {\n  // use \"c.satz\"\n}\n") && s.starts_with("// why")));
        assert_eq!(block_stub("google_folder.infra_folder", "// use \"c.satz\"", ""), None, "a folder is the estate's own structure");
    }

    #[test]
    fn the_skeleton_carries_one_use_line_per_choice_the_map_declares() {
        // Two places for one list — the map declares the choices, the skeleton carries their
        // `use … when` lines. This is what keeps them equal.
        let map = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/presets/estate-map.satz")).unwrap();
        let map = satz_core::satz::parse(&map).unwrap();
        let sk = skeleton("x", Some(&shipped())).unwrap();
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
        let sk = skeleton("x", Some(&shipped())).unwrap();
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
        let written = [("the interview skeleton", skeleton("x", Some(&graph)).unwrap()), ("the init estate", std::fs::read_to_string(&init).unwrap())];
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
            "a pack line before the line of a pack whose param it reads — answering it yes stops the compile with `unknown param`. A top-level pack that reads a param of a pack inside the scaffold is placed `after_scaffold`:\n  {}",
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

    #[test]
    fn estate_names_are_identifiers() {
        assert_eq!(estate_name("C0example"), "c0example");
        assert_eq!(estate_name("0abc-x"), "_0abc_x");
    }
}
