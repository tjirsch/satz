//! What `satz init` writes, and where each value came from.
//!
//! Three sources, in this order and no other: stated on the command line, derived
//! from the Application Default Credentials, or EMPTY. There is no fourth — init
//! used to substitute `"123456789012"` for an unknown organisation and
//! `"first.admin"` for an unknown admin, which produced a file that looked
//! complete and pointed `satz bootstrap` at an organisation nobody owned.
//!
//! And a re-run MERGES: the params named on this command line are written into
//! the estate that already exists, every other line is left exactly as it is.
//! Skipping it silently is what made an operator believe a value had landed when
//! it had not.

/// The values the operator typed, before anything was derived. A re-run writes
/// exactly these, so a derived value never overwrites a file somebody has since
/// edited by hand.
#[derive(Debug, Default, Clone)]
pub(crate) struct Stated {
    pub customer_id: Option<String>,
    pub customer_shortname: Option<String>,
    pub billing_account_infra: Option<String>,
    pub default_region: Option<String>,
    pub customer_organization_id: Option<String>,
    pub customer_domain: Option<String>,
    pub infra_project_name: Option<String>,
    pub infra_bucket_name: Option<String>,
    /// `--iac-user` is `<local>@<domain>`; the estate binds the local part.
    pub iac_user: Option<String>,
}

impl Stated {
    /// `(param, value)` for every param this command line named, in the order
    /// the estate writes them. `--iac-user` becomes `first_admin`, its local
    /// part, because that is what the packs compose members from.
    pub(crate) fn params(&self) -> Vec<(&'static str, String)> {
        let mut out: Vec<(&'static str, String)> = Vec::new();
        let mut push = |name: &'static str, v: &Option<String>| {
            if let Some(v) = v {
                out.push((name, v.clone()));
            }
        };
        push("customer_id", &self.customer_id);
        push("customer_organization_id", &self.customer_organization_id);
        push("customer_domain", &self.customer_domain);
        push("customer_shortname", &self.customer_shortname);
        push("billing_account_infra", &self.billing_account_infra);
        push("infra_project_name", &self.infra_project_name);
        push("infra_bucket_name", &self.infra_bucket_name);
        push("default_region", &self.default_region);
        if let Some(user) = &self.iac_user {
            let local = user.split('@').next().unwrap_or(user);
            out.push(("first_admin", local.to_string()));
        }
        out
    }
}

/// What was derived and from where, so the operator can catch a wrong tenant on
/// the line that reports it rather than on the first apply.
#[derive(Debug, Default)]
pub(crate) struct Derivations {
    lines: Vec<String>,
    empty: Vec<&'static str>,
}

impl Derivations {
    /// `stated` wins untouched. Otherwise take `derived` when it has a value and
    /// say where it came from; when it has none, the param stays empty and is
    /// named at the end. Never a placeholder.
    pub(crate) fn fill(
        &mut self,
        param: &'static str,
        stated: Option<String>,
        derived: Option<String>,
        source: &str,
    ) -> Option<String> {
        if stated.is_some() {
            return stated;
        }
        match derived.filter(|v| !v.trim().is_empty()) {
            Some(v) => {
                self.lines.push(format!("  {} = {} (derived from {})", param, v, source));
                Some(v)
            }
            None => {
                self.empty.push(param);
                None
            }
        }
    }

    pub(crate) fn report(&self) {
        if !self.lines.is_empty() {
            println!("init: derived from your credentials —");
            for l in &self.lines {
                println!("{}", l);
            }
        }
        if !self.empty.is_empty() {
            println!(
                "init: nothing could answer {} — written empty, and `satz bootstrap` refuses by name until they are set",
                self.empty.join(", ")
            );
        }
    }
}

/// The literal a param is bound to in an estate's `params` block, if it is bound
/// at all. Textual on purpose: this runs on a file that may not compile yet.
pub(crate) fn current_value(src: &str, param: &str) -> Option<String> {
    for line in src.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix(param) else { continue };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        // a quoted literal; anything else (a reference, an interpolation) is
        // reported as-is up to a trailing comment
        if let Some(inner) = rest.strip_prefix('"') {
            return inner.split('"').next().map(str::to_string);
        }
        return Some(rest.split("//").next().unwrap_or(rest).trim().to_string());
    }
    None
}

/// One param's fate in a merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Merged {
    /// Was `from`, is now `to`.
    Changed { param: &'static str, from: String, to: String },
    /// Already bound to this value.
    Same { param: &'static str, value: String },
}

/// Apply the stated params to an existing estate, one `bind` each, and say what
/// each one did. The file keeps its layout, its comments and every param this
/// command line did not name.
pub(crate) fn merge(src: &str, stated: &Stated) -> Result<(String, Vec<Merged>), String> {
    let mut out = src.to_string();
    let mut log = Vec::new();
    for (param, value) in stated.params() {
        let before = current_value(&out, param).unwrap_or_default();
        if before == value {
            log.push(Merged::Same { param, value });
            continue;
        }
        out = crate::interview::bind(&out, param, &serde_yaml::Value::String(value.clone()))?;
        log.push(Merged::Changed { param, from: before, to: value });
    }
    Ok((out, log))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESTATE: &str = r#"estate c0example

params {
  customer_id              = "C0example"
  customer_organization_id = ""
  customer_domain          = "example.com" // the primary domain
  customer_shortname       = "acme"
  billing_account_infra    = ""
  infra_project_name       = "acme-infra-001"
  default_region           = "europe-west3"
}
"#;

    fn stated_billing_and_org() -> Stated {
        Stated {
            billing_account_infra: Some("012345-6789AB-CDEF01".into()),
            customer_organization_id: Some("123456789012".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_rerun_writes_what_was_named_and_leaves_everything_else() {
        let (out, log) = merge(ESTATE, &stated_billing_and_org()).unwrap();
        assert!(out.contains(r#"billing_account_infra    = "012345-6789AB-CDEF01""#), "{}", out);
        assert!(out.contains(r#"customer_organization_id = "123456789012""#), "{}", out);
        // untouched, comment and all
        assert!(out.contains(r#"customer_domain          = "example.com" // the primary domain"#), "{}", out);
        assert!(out.contains(r#"customer_shortname       = "acme""#), "{}", out);
        assert_eq!(
            log,
            vec![
                Merged::Changed { param: "customer_organization_id", from: String::new(), to: "123456789012".into() },
                Merged::Changed { param: "billing_account_infra", from: String::new(), to: "012345-6789AB-CDEF01".into() },
            ]
        );
    }

    #[test]
    fn a_value_that_is_already_right_is_reported_as_kept_not_rewritten() {
        let stated = Stated { customer_shortname: Some("acme".into()), ..Default::default() };
        let (out, log) = merge(ESTATE, &stated).unwrap();
        assert_eq!(out, ESTATE, "nothing was rewritten");
        assert_eq!(log, vec![Merged::Same { param: "customer_shortname", value: "acme".into() }]);
    }

    #[test]
    fn a_param_the_estate_does_not_have_yet_is_added() {
        let stated = Stated { infra_bucket_name: Some("acme-infra-001-state".into()), ..Default::default() };
        let (out, log) = merge(ESTATE, &stated).unwrap();
        assert!(out.contains("infra_bucket_name"), "{}", out);
        assert_eq!(log.len(), 1);
        assert!(matches!(&log[0], Merged::Changed { param: "infra_bucket_name", from, .. } if from.is_empty()));
    }

    #[test]
    fn the_admin_address_is_split_to_the_local_part_the_packs_compose_from() {
        let stated = Stated { iac_user: Some("alice@example.com".into()), ..Default::default() };
        assert_eq!(stated.params(), vec![("first_admin", "alice".to_string())]);
        let bare = Stated { iac_user: Some("alice".into()), ..Default::default() };
        assert_eq!(bare.params(), vec![("first_admin", "alice".to_string())]);
    }

    #[test]
    fn current_value_reads_a_binding_without_compiling_the_file() {
        assert_eq!(current_value(ESTATE, "customer_shortname").as_deref(), Some("acme"));
        assert_eq!(current_value(ESTATE, "customer_organization_id").as_deref(), Some(""));
        assert_eq!(current_value(ESTATE, "customer_domain").as_deref(), Some("example.com"), "the comment is not the value");
        assert_eq!(current_value(ESTATE, "infra_bucket_name"), None);
        // a reference rather than a literal
        assert_eq!(current_value("params {\n  a = other_param\n}\n", "a").as_deref(), Some("other_param"));
    }

    #[test]
    fn stated_wins_over_derived_and_nothing_is_ever_invented() {
        let mut d = Derivations::default();
        assert_eq!(d.fill("a", Some("typed".into()), Some("derived".into()), "src").as_deref(), Some("typed"));
        assert_eq!(d.fill("b", None, Some("derived".into()), "src").as_deref(), Some("derived"));
        assert_eq!(d.fill("c", None, None, "src"), None);
        assert_eq!(d.fill("d", None, Some("   ".into()), "src"), None, "a blank derivation is no derivation");
        assert_eq!(d.lines.len(), 1, "{:?}", d.lines);
        assert!(d.lines[0].contains("b = derived (derived from src)"), "{:?}", d.lines);
        assert_eq!(d.empty, vec!["c", "d"]);
    }
}
