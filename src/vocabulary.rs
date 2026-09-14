//! The day-0 vocabulary of a discovered estate: the params `satz init` writes
//! (`presets/estate-core.satz`), bound from what the platform states and what
//! the discovered data implies, so a discovered estate lines up with a written
//! one and the packs drop in. Three classes, each visible in the file:
//! DERIVED — a fact the platform states, bound without comment; INFERRED — a
//! rule over the discovered data, bound with a `// inferred:` note naming the
//! rule and its evidence, and one report line; NOT DERIVABLE — absent from
//! `params`, reported with how to bind it. Never an empty string standing in
//! for a value, never a placeholder. ADR 0020.

use std::collections::BTreeMap;

use crate::config::{Config, Folder, Project};
use satz_core::condense::Substitution;

/// What the ADC states about the tenant — `init --from-live`'s derivation,
/// reused by the live shape. The state shape has none of it.
#[derive(Debug, Default, Clone)]
pub struct LiveFacts {
    pub customer_id: Option<String>,
    pub customer_domain: Option<String>,
    pub first_admin: Option<String>,
    pub billing_account: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    Derived,
    /// The rule and its evidence, as written beside the binding.
    Inferred(String),
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub name: &'static str,
    pub value: String,
    pub class: Class,
}

#[derive(Debug, Default)]
pub struct Vocabulary {
    /// In the library's order.
    pub bindings: Vec<Binding>,
    /// `(param, how to bind it)`.
    pub not_derivable: Vec<(&'static str, String)>,
    /// Projects that carry the bound billing account (the attribute goes) and
    /// projects that carry none (they keep `billing_account = ""`).
    billing_dropped: usize,
    billing_none: Vec<String>,
}

const ORDER: [&str; 16] = [
    "customer_id",
    "customer_organization_id",
    "customer_domain",
    "customer_shortname",
    "first_admin",
    "billing_account_infra",
    "customer_longname",
    "infra_folder_name",
    "infra_project_name",
    "infra_bucket_name",
    "svc_iac_account",
    "svc_iac_users_group",
    "deployment_engine",
    "deployment_mode",
    "default_region",
    "default_zone",
];

/// The params whose literal value is referenced wherever the document repeats
/// it. Deployment mode and engine name nothing in the data; the long name is
/// never bound.
const REFERENCED: [&str; 12] = [
    "customer_id",
    "customer_domain",
    "customer_shortname",
    "first_admin",
    "billing_account_infra",
    "infra_folder_name",
    "infra_project_name",
    "infra_bucket_name",
    "svc_iac_account",
    "svc_iac_users_group",
    "default_region",
    "default_zone",
];

const SA_SUFFIX: &str = ".iam.gserviceaccount.com";

struct ProjectFacts {
    id: String,
    billing: Option<String>,
    /// `(bucket name, versioning on)`
    buckets: Vec<(String, bool)>,
    /// display name of the folder holding it; `None` at the top level
    folder: Option<String>,
}

#[derive(Default)]
struct Facts {
    projects: Vec<ProjectFacts>,
    /// member → roles, at the organization
    org_grants: Vec<(String, Vec<String>)>,
    /// every member granted anywhere
    members: Vec<String>,
    /// region → how many regional resources name it
    regions: BTreeMap<String, usize>,
}

fn grant_roles(roles: &serde_yaml::Value) -> Vec<String> {
    roles
        .as_sequence()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().or_else(|| r.as_mapping().and_then(|m| m.get("role")).and_then(|v| v.as_str())))
        .map(String::from)
        .collect()
}

fn is_region(s: &str) -> bool {
    let mut parts = s.split('-');
    matches!(
        (parts.next(), parts.next(), parts.next()),
        (Some(a), Some(b), None)
            if a.chars().all(|c| c.is_ascii_lowercase())
                && b.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && b.chars().last().is_some_and(|c| c.is_ascii_digit())
    )
}

fn collect_regions(v: &serde_yaml::Value, key: Option<&str>, regions: &mut BTreeMap<String, usize>) {
    match v {
        serde_yaml::Value::String(s) if matches!(key, Some("location" | "region")) && is_region(s) => {
            *regions.entry(s.clone()).or_default() += 1;
        }
        serde_yaml::Value::Sequence(items) => {
            for i in items {
                collect_regions(i, None, regions);
            }
        }
        serde_yaml::Value::Mapping(m) => {
            for (k, i) in m {
                collect_regions(i, k.as_str(), regions);
            }
        }
        _ => {}
    }
}

fn collect_extra(extra: &std::collections::HashMap<String, serde_yaml::Value>, facts: &mut Facts) {
    for (tf_type, val) in extra {
        if tf_type.ends_with("_iam_member") {
            if let Some(m) = val.as_mapping() {
                for (k, v) in m {
                    if let (Some(member), serde_yaml::Value::Sequence(_)) = (k.as_str(), v) {
                        facts.members.push(member.to_string());
                    }
                }
            }
            continue;
        }
        collect_regions(val, None, &mut facts.regions);
    }
}

fn collect_project(p: &Project, folder: Option<&str>, facts: &mut Facts) {
    let buckets = p
        .extra
        .get("google_storage_bucket")
        .and_then(|v| v.as_mapping())
        .into_iter()
        .flatten()
        .filter_map(|(label, body)| {
            let body = body.as_mapping()?;
            let name = body.get("name").and_then(|v| v.as_str()).or(label.as_str())?.to_string();
            let versioning = match body.get("versioning") {
                Some(serde_yaml::Value::Mapping(m)) => m.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
                Some(serde_yaml::Value::Sequence(s)) => {
                    s.first().and_then(|v| v.as_mapping()).and_then(|m| m.get("enabled")).and_then(|v| v.as_bool()).unwrap_or(false)
                }
                _ => false,
            };
            Some((name, versioning))
        })
        .collect();
    facts.projects.push(ProjectFacts { id: p.project_id.clone(), billing: p.billing_account.clone(), buckets, folder: folder.map(String::from) });
    collect_extra(&p.extra, facts);
}

fn collect_folder(f: &Folder, facts: &mut Facts) {
    collect_extra(&f.extra, facts);
    for p in f.project.iter().flat_map(|m| m.values()) {
        collect_project(p, Some(&f.display_name), facts);
    }
    for sub in f.folder.iter().flat_map(|m| m.values()) {
        collect_folder(sub, facts);
    }
}

fn facts_of(config: &Config) -> Facts {
    let mut facts = Facts::default();
    for (member, roles) in config.organization_iam_member.iter().flatten() {
        facts.members.push(member.clone());
        facts.org_grants.push((member.clone(), grant_roles(&serde_yaml::Value::Sequence(roles.clone()))));
    }
    collect_extra(&config.extra, &mut facts);
    for p in config.project.iter().flat_map(|m| m.values()) {
        collect_project(p, None, &mut facts);
    }
    for f in config.folder.iter().flat_map(|m| m.values()) {
        collect_folder(f, &mut facts);
    }
    facts
}

/// The one value with the strictly highest count, with `(count, total)`.
fn unique_max(counts: &BTreeMap<String, usize>) -> Option<(String, usize, usize)> {
    let total: usize = counts.values().sum();
    let max = *counts.values().max()?;
    let mut at_max = counts.iter().filter(|(_, n)| **n == max);
    let (v, n) = at_max.next()?;
    if at_max.next().is_some() {
        return None;
    }
    Some((v.clone(), *n, total))
}

impl Vocabulary {
    /// Bind the vocabulary from the discovered estate, the ADC's facts (live
    /// shape) and the explicit short name.
    pub fn infer(config: &Config, organization: Option<&str>, live: Option<&LiveFacts>, shortname: Option<&str>) -> Vocabulary {
        let facts = facts_of(config);
        let mut v = Vocabulary::default();
        let derived = |v: &mut Vocabulary, name: &'static str, value: &str| {
            v.bindings.push(Binding { name, value: value.to_string(), class: Class::Derived });
        };
        let inferred = |v: &mut Vocabulary, name: &'static str, value: &str, rule: String| {
            v.bindings.push(Binding { name, value: value.to_string(), class: Class::Inferred(rule) });
        };

        // --- what the platform states
        match live.and_then(|l| l.customer_id.as_deref()) {
            Some(id) => derived(&mut v, "customer_id", id),
            None => v.not_derivable.push(("customer_id", "the directory customer id (C0…); the live shape reads it from the ADC, `satz interview` asks".into())),
        }
        if let Some(org) = organization.filter(|o| !o.is_empty()) {
            derived(&mut v, "customer_organization_id", org);
        }
        match live.and_then(|l| l.customer_domain.as_deref()) {
            Some(d) => derived(&mut v, "customer_domain", d),
            None => {
                let mut domains: BTreeMap<String, usize> = BTreeMap::new();
                for m in &facts.members {
                    if let Some(rest) = m.strip_prefix("user:").or_else(|| m.strip_prefix("group:")) {
                        if let Some((_, domain)) = rest.rsplit_once('@') {
                            *domains.entry(domain.to_string()).or_default() += 1;
                        }
                    }
                }
                match unique_max(&domains) {
                    Some((d, n, total)) => inferred(&mut v, "customer_domain", &d, format!("the domain of {} of {} user and group members", n, total)),
                    None => v.not_derivable.push(("customer_domain", "no single domain leads among the members; the live shape reads it from the organization".into())),
                }
            }
        }

        // --- the short name: no platform fact carries it
        match shortname {
            Some(s) => derived(&mut v, "customer_shortname", s),
            None => {
                let mut tokens: BTreeMap<String, usize> = BTreeMap::new();
                let names = facts.projects.iter().map(|p| p.id.as_str()).chain(facts.projects.iter().flat_map(|p| p.buckets.iter().map(|(b, _)| b.as_str())));
                let mut total = 0;
                for name in names {
                    total += 1;
                    if let Some((head, _)) = name.split_once('-') {
                        if head.chars().next().is_some_and(|c| c.is_ascii_lowercase()) && head.chars().all(|c| c.is_ascii_alphanumeric()) {
                            *tokens.entry(head.to_string()).or_default() += 1;
                        }
                    }
                }
                match unique_max(&tokens) {
                    Some((t, n, _)) if n >= 2 => inferred(&mut v, "customer_shortname", &t, format!("the leading token of {} of {} project and bucket names", n, total)),
                    Some(_) | None if tokens.values().max().is_some_and(|m| *m >= 2) => {
                        let max = tokens.values().max().copied().unwrap_or(0);
                        let tied: Vec<String> = tokens.iter().filter(|(_, n)| **n == max).map(|(t, n)| format!("{} ({})", t, n)).collect();
                        v.not_derivable.push(("customer_shortname", format!("the leading name tokens tie: {}; pass --customer-shortname", tied.join(", "))));
                    }
                    _ => v.not_derivable.push(("customer_shortname", "no leading name token repeats; pass --customer-shortname".into())),
                }
            }
        }

        match live.and_then(|l| l.first_admin.as_deref()) {
            Some(a) => derived(&mut v, "first_admin", a),
            None => v.not_derivable.push(("first_admin", "the identity running the import; the live shape reads it from the ADC".into())),
        }

        // --- the IaC service account and everything that follows from it
        let admins: Vec<&str> = facts
            .org_grants
            .iter()
            .filter(|(m, roles)| {
                m.starts_with("serviceAccount:")
                    && (roles.iter().any(|r| r == "roles/resourcemanager.organizationAdmin")
                        || (roles.iter().any(|r| r == "roles/resourcemanager.folderAdmin") && roles.iter().any(|r| r == "roles/resourcemanager.projectCreator")))
            })
            .map(|(m, _)| m.as_str())
            .collect();
        let mut infra: Option<(String, String)> = None; // (account local part, project id)
        match admins.as_slice() {
            [one] => {
                let email = one.trim_start_matches("serviceAccount:");
                match email.split_once('@').and_then(|(local, host)| host.strip_suffix(SA_SUFFIX).map(|project| (local, project))) {
                    Some((local, project)) => {
                        let rule = format!("the one service account granted organizationAdmin at the organization, {}", email);
                        inferred(&mut v, "svc_iac_account", local, rule.clone());
                        inferred(&mut v, "infra_project_name", project, format!("the project of {}", email));
                        infra = Some((local.to_string(), project.to_string()));
                    }
                    None => v.not_derivable.push(("svc_iac_account", format!("{} is not a project service account", email))),
                }
            }
            [] => v.not_derivable.push(("svc_iac_account", "no service account holds roles/resourcemanager.organizationAdmin at the organization".into())),
            many => v.not_derivable.push(("svc_iac_account", format!("{} service accounts hold organizationAdmin at the organization: {}", many.len(), many.join(", ")))),
        }
        let infra_project = infra.as_ref().and_then(|(_, id)| facts.projects.iter().find(|p| &p.id == id));
        match (&infra, infra_project) {
            (Some((_, id)), None) => {
                v.not_derivable.push(("infra_folder_name", format!("the infra project {} is not in the sweep", id)));
                v.not_derivable.push(("infra_bucket_name", format!("the infra project {} is not in the sweep", id)));
            }
            (Some(_), Some(p)) => {
                match &p.folder {
                    Some(f) => inferred(&mut v, "infra_folder_name", f, format!("the folder holding the infra project {}", p.id)),
                    None => inferred(&mut v, "infra_folder_name", "", format!("the infra project {} sits at the top level", p.id)),
                }
                let versioned: Vec<&str> = p.buckets.iter().filter(|(_, on)| *on).map(|(b, _)| b.as_str()).collect();
                match (versioned.as_slice(), p.buckets.as_slice()) {
                    ([one], _) => inferred(&mut v, "infra_bucket_name", one, format!("the one versioned bucket in the infra project {}", p.id)),
                    ([], [(one, _)]) => inferred(&mut v, "infra_bucket_name", one, format!("the one bucket in the infra project {}", p.id)),
                    ([], []) => v.not_derivable.push(("infra_bucket_name", format!("the infra project {} has no bucket", p.id))),
                    (many, _) if !many.is_empty() => v.not_derivable.push(("infra_bucket_name", format!("{} versioned buckets in the infra project {}: {}", many.len(), p.id, many.join(", ")))),
                    (_, all) => v.not_derivable.push(("infra_bucket_name", format!("{} buckets in the infra project {}, none versioned", all.len(), p.id))),
                }
            }
            (None, _) => {
                v.not_derivable.push(("infra_folder_name", "follows from the infra project".into()));
                v.not_derivable.push(("infra_bucket_name", "follows from the infra project".into()));
            }
        }

        // --- billing: the infra project's account, else the single open one
        let billing = infra_project.and_then(|p| p.billing.clone()).map(|b| (b, Class::Inferred("the infra project's billing account".into())));
        let billing = billing.or_else(|| live.and_then(|l| l.billing_account.clone()).map(|b| (b, Class::Derived)));
        match billing {
            Some((b, class)) => {
                v.bindings.push(Binding { name: "billing_account_infra", value: b.clone(), class });
                for p in &facts.projects {
                    match &p.billing {
                        Some(pb) if *pb == b => v.billing_dropped += 1,
                        None => v.billing_none.push(p.id.clone()),
                        Some(_) => {}
                    }
                }
            }
            None => v.not_derivable.push(("billing_account_infra", "no infra project account and no single open billing account".into())),
        }

        v.not_derivable.push(("customer_longname", "nothing on the platform names it; `satz interview` asks".into()));

        // --- the IaC users group
        match &infra {
            Some((local, _)) => {
                let wanted = format!("{}-users", local);
                let group = facts.members.iter().find_map(|m| {
                    let rest = m.strip_prefix("group:")?;
                    let (name, _) = rest.split_once('@')?;
                    (name == wanted).then_some(rest.to_string())
                });
                match group {
                    Some(email) => inferred(&mut v, "svc_iac_users_group", &wanted, format!("the group {} among the members", email)),
                    None => v.not_derivable.push(("svc_iac_users_group", format!("no group named {} among the members", wanted))),
                }
            }
            None => v.not_derivable.push(("svc_iac_users_group", "follows from the IaC service account".into())),
        }

        derived(&mut v, "deployment_engine", "tofu");
        derived(&mut v, "deployment_mode", "local");

        // --- the region the estate lives in
        match unique_max(&facts.regions) {
            Some((r, n, total)) => {
                inferred(&mut v, "default_region", &r, format!("the region of {} of {} regional resources", n, total));
                inferred(&mut v, "default_zone", "{default_region}-a", "the region's first zone".into());
            }
            None => {
                v.not_derivable.push(("default_region", "no single region leads among the regional resources".into()));
                v.not_derivable.push(("default_zone", "follows from the region".into()));
            }
        }

        v.bindings.sort_by_key(|b| ORDER.iter().position(|n| *n == b.name).unwrap_or(ORDER.len()));
        v
    }

    fn value_of(&self, name: &str) -> Option<&str> {
        self.bindings.iter().find(|b| b.name == name).map(|b| b.value.as_str())
    }

    /// The `params` block, rendered: `(name, value)` pairs where an inferred
    /// value carries its `// inferred:` note on the line.
    pub fn params(&self) -> Vec<(String, String)> {
        self.bindings
            .iter()
            .map(|b| {
                let rendered = match &b.class {
                    Class::Derived => format!("\"{}\"", b.value),
                    Class::Inferred(rule) => format!("\"{}\" // inferred: {}", b.value, rule),
                };
                (b.name.to_string(), rendered)
            })
            .collect()
    }

    /// The literals to reference wherever the document repeats them.
    pub fn substitutions(&self) -> Vec<Substitution> {
        let mut out: Vec<Substitution> = self
            .bindings
            .iter()
            .filter(|b| REFERENCED.contains(&b.name) && b.value.len() >= 3 && !b.value.contains('{'))
            .map(|b| Substitution { literal: b.value.clone(), param: b.name.to_string() })
            .collect();
        // the zone is bound as an interpolation; its literal is the region's
        if let Some(region) = self.value_of("default_region") {
            out.push(Substitution { literal: format!("{}-a", region), param: "default_zone".into() });
        }
        out
    }

    /// Drop the per-project `billing_account` where it is the bound one; a
    /// project without any keeps an explicit empty string, or the emitter's
    /// fallback would attach the bound account on apply.
    pub fn apply_billing(&self, config: &mut Config) {
        let Some(bound) = self.value_of("billing_account_infra").map(String::from) else { return };
        fn each<'a>(f: &'a mut Folder, out: &mut Vec<&'a mut Project>) {
            if let Some(ps) = &mut f.project {
                out.extend(ps.values_mut());
            }
            if let Some(fs) = &mut f.folder {
                for sub in fs.values_mut() {
                    each(sub, out);
                }
            }
        }
        let mut projects: Vec<&mut Project> = Vec::new();
        if let Some(ps) = &mut config.project {
            projects.extend(ps.values_mut());
        }
        if let Some(fs) = &mut config.folder {
            for f in fs.values_mut() {
                each(f, &mut projects);
            }
        }
        for p in projects {
            match &p.billing_account {
                Some(b) if *b == bound => p.billing_account = None,
                None => p.billing_account = Some(String::new()),
                Some(_) => {}
            }
        }
    }

    /// The report, one line per class.
    pub fn report(&self) -> Vec<String> {
        let mut out = Vec::new();
        let derived: Vec<&str> = self.bindings.iter().filter(|b| b.class == Class::Derived).map(|b| b.name).collect();
        if !derived.is_empty() {
            out.push(format!("import: params derived: {}", derived.join(", ")));
        }
        for b in &self.bindings {
            if let Class::Inferred(rule) = &b.class {
                out.push(format!("import: params inferred: {} = \"{}\" — {}", b.name, b.value, rule));
            }
        }
        for (name, how) in &self.not_derivable {
            out.push(format!("import: params not derivable: {} — {}", name, how));
        }
        if self.billing_dropped > 0 || !self.billing_none.is_empty() {
            let mut line = format!("import: billing: {} project(s) carry billing_account_infra and drop the attribute", self.billing_dropped);
            if !self.billing_none.is_empty() {
                line.push_str(&format!("; {} without an account keep `billing_account = \"\"`: {}", self.billing_none.len(), self.billing_none.join(", ")));
            }
            out.push(line);
        }
        out
    }
}

#[cfg(test)]
mod vocabulary_tests {
    //! One rule per binding, over a synthetic discovered estate: what the
    //! sweep can identify with certainty is bound and marked, what it cannot
    //! is named.
    use super::*;

    const ESTATE: &str = r#"
organization_iam_member:
  "serviceAccount:svc-iac-001@acme-infra-001.iam.gserviceaccount.com":
    - role: roles/resourcemanager.organizationAdmin
    - role: roles/resourcemanager.folderAdmin
  "group:svc-iac-001-users@example.com":
    - roles/browser
  "user:alice@example.com":
    - roles/viewer
  "user:bob@example.com":
    - roles/viewer
google_logging_organization_sink:
  audit:
    name: audit
    destination: storage.googleapis.com/acme-organization-audit-logs
folder:
  infra_folder:
    display_name: Infrastructure
    project:
      infra:
        project_id: acme-infra-001
        billing_account: 01AA-BB-CC
        google_storage_bucket:
          state:
            name: acme-infra-001-state
            location: EU
            versioning:
              enabled: true
          logs:
            name: acme-logs-001
            location: europe-west3
project:
  log:
    project_id: acme-log-001
    billing_account: 01AA-BB-CC
    google_pubsub_topic:
      t:
        name: t
  probe:
    project_id: probe-x7k2
    google_compute_subnetwork:
      s:
        name: s
        region: europe-west3
"#;

    fn estate() -> Config {
        serde_yaml::from_str(ESTATE).unwrap()
    }

    fn binding<'a>(v: &'a Vocabulary, name: &str) -> &'a Binding {
        v.bindings.iter().find(|b| b.name == name).unwrap_or_else(|| panic!("{} is not bound: {:?}", name, v.not_derivable))
    }

    #[test]
    fn the_state_shape_infers_what_the_data_carries_and_names_the_rest() {
        let v = Vocabulary::infer(&estate(), Some("123456789012"), None, None);
        assert_eq!(binding(&v, "customer_organization_id").class, Class::Derived);
        assert_eq!(binding(&v, "svc_iac_account").value, "svc-iac-001");
        assert_eq!(binding(&v, "infra_project_name").value, "acme-infra-001");
        assert_eq!(binding(&v, "infra_folder_name").value, "Infrastructure");
        assert_eq!(binding(&v, "infra_bucket_name").value, "acme-infra-001-state");
        assert_eq!(binding(&v, "billing_account_infra").value, "01AA-BB-CC");
        assert_eq!(binding(&v, "svc_iac_users_group").value, "svc-iac-001-users");
        assert_eq!(binding(&v, "customer_shortname").value, "acme", "3 of 5 names lead with it");
        assert_eq!(binding(&v, "customer_domain").value, "example.com");
        assert!(matches!(&binding(&v, "customer_domain").class, Class::Inferred(r) if r.contains("3 of 3")), "{:?}", v.bindings);
        assert_eq!(binding(&v, "default_region").value, "europe-west3");
        assert_eq!(binding(&v, "default_zone").value, "{default_region}-a");
        assert_eq!(binding(&v, "deployment_mode").value, "local");
        let missing: Vec<&str> = v.not_derivable.iter().map(|(n, _)| *n).collect();
        assert_eq!(missing, ["customer_id", "first_admin", "customer_longname"], "{:?}", v.not_derivable);
        let names: Vec<&str> = v.bindings.iter().map(|b| b.name).collect();
        assert_eq!(names[0], "customer_organization_id", "the library's order: {:?}", names);
        assert_eq!(names.last(), Some(&"default_zone"));
    }

    #[test]
    fn the_live_shape_takes_the_platform_facts_as_derived_and_the_flag_wins() {
        let live = LiveFacts {
            customer_id: Some("C0example".into()),
            customer_domain: Some("example.org".into()),
            first_admin: Some("carol".into()),
            billing_account: Some("99ZZ-YY-XX".into()),
        };
        let v = Vocabulary::infer(&estate(), Some("123456789012"), Some(&live), Some("corp"));
        assert_eq!(binding(&v, "customer_id").class, Class::Derived);
        assert_eq!(binding(&v, "customer_domain").value, "example.org", "the organization's word beats the members' domains");
        assert_eq!(binding(&v, "first_admin").value, "carol");
        assert_eq!(binding(&v, "customer_shortname").value, "corp");
        assert_eq!(binding(&v, "customer_shortname").class, Class::Derived);
        assert_eq!(binding(&v, "billing_account_infra").value, "01AA-BB-CC", "the infra project's account beats the ADC's single one");
        assert_eq!(v.not_derivable.iter().map(|(n, _)| *n).collect::<Vec<_>>(), ["customer_longname"]);
    }

    #[test]
    fn two_admin_service_accounts_are_named_and_nothing_downstream_is_guessed() {
        let mut config = estate();
        config.organization_iam_member.as_mut().unwrap().insert(
            "serviceAccount:svc-other@acme-infra-001.iam.gserviceaccount.com".into(),
            vec![serde_yaml::Value::String("roles/resourcemanager.organizationAdmin".into())],
        );
        let v = Vocabulary::infer(&config, None, None, None);
        let (_, how) = v.not_derivable.iter().find(|(n, _)| *n == "svc_iac_account").unwrap();
        assert!(how.contains("2 service accounts") && how.contains("svc-other"), "{}", how);
        for name in ["infra_project_name", "infra_folder_name", "infra_bucket_name", "svc_iac_users_group"] {
            assert!(v.bindings.iter().all(|b| b.name != name), "{} must not be guessed", name);
        }
        assert!(v.bindings.iter().all(|b| b.name != "billing_account_infra"), "no infra project, no ADC: not bound");
    }

    #[test]
    fn params_render_the_note_and_substitutions_reference_the_literals() {
        let v = Vocabulary::infer(&estate(), Some("123456789012"), None, None);
        let params = v.params();
        let (_, shortname) = params.iter().find(|(n, _)| n == "customer_shortname").unwrap();
        assert!(shortname.starts_with("\"acme\" // inferred: the leading token"), "{}", shortname);
        let (_, org) = params.iter().find(|(n, _)| n == "customer_organization_id").unwrap();
        assert_eq!(org, "\"123456789012\"");
        let (_, zone) = params.iter().find(|(n, _)| n == "default_zone").unwrap();
        assert!(zone.starts_with("\"{default_region}-a\""), "{}", zone);
        let subs = v.substitutions();
        let literal = |p: &str| subs.iter().find(|s| s.param == p).map(|s| s.literal.clone());
        assert_eq!(literal("default_zone").as_deref(), Some("europe-west3-a"));
        assert_eq!(literal("infra_project_name").as_deref(), Some("acme-infra-001"));
        assert_eq!(literal("customer_shortname").as_deref(), Some("acme"));
        assert_eq!(literal("deployment_mode"), None, "nothing in the data is a deployment mode");
        assert_eq!(literal("customer_organization_id"), None, "the organization is the pass's own rule");
    }

    #[test]
    fn a_tie_between_leading_tokens_is_named_and_binds_nothing() {
        let mut config = estate();
        // a second family of names as strong as the first
        config.project.as_mut().unwrap().insert("corp".into(), Project { project_id: "corp-log-001".into(), ..Default::default() });
        config.project.as_mut().unwrap().insert("corp2".into(), Project { project_id: "corp-app-001".into(), ..Default::default() });
        config.project.as_mut().unwrap().insert("corp3".into(), Project { project_id: "corp-x-001".into(), ..Default::default() });
        config.project.as_mut().unwrap().insert("corp4".into(), Project { project_id: "corp-y-001".into(), ..Default::default() });
        let v = Vocabulary::infer(&config, None, None, None);
        let (_, how) = v.not_derivable.iter().find(|(n, _)| *n == "customer_shortname").unwrap_or_else(|| panic!("{:?}", v.bindings));
        assert!(how.contains("tie: acme (4), corp (4)"), "{}", how);
        assert!(v.bindings.iter().all(|b| b.name != "customer_shortname"));
    }

    #[test]
    fn billing_drops_where_bound_and_stays_empty_where_none() {
        let mut config = estate();
        let v = Vocabulary::infer(&config, None, None, None);
        v.apply_billing(&mut config);
        let infra = &config.folder.as_ref().unwrap()["infra_folder"].project.as_ref().unwrap()["infra"];
        assert_eq!(infra.billing_account, None);
        let probe = &config.project.as_ref().unwrap()["probe"];
        assert_eq!(probe.billing_account.as_deref(), Some(""), "explicit: the emitter's fallback must not attach one");
        let report = v.report().join("\n");
        assert!(report.contains("2 project(s) carry billing_account_infra"), "{}", report);
        assert!(report.contains("probe-x7k2"), "{}", report);
        assert!(report.contains("params not derivable: customer_id"), "{}", report);
        assert!(report.contains("params inferred: svc_iac_account = \"svc-iac-001\""), "{}", report);
    }
}
