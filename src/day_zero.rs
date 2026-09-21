//! The gate every bootstrap passes before it touches an API.
//!
//! An empty or malformed day-0 param used to reach Google as part of a URL:
//! `billingAccounts/:testIamPermissions` — note the empty segment — came back as
//! a 404 HTML page that satz dumped into an error string, `<!DOCTYPE html>` and
//! all. The worse shape is silent: a plausible-looking organisation id nobody
//! owns produces a perfectly ordinary pre-flight against the wrong organisation.
//!
//! So: every param bootstrap consumes is checked here for presence and shape,
//! the failure names the param AND the flag that sets it, and no credential is
//! asked for until this passes.

/// One thing wrong with the estate, as the operator has to fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Problem {
    pub param: &'static str,
    pub flag: &'static str,
    pub says: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "  {} — {}\n      set it with `satz init --{}`", self.param, self.says, self.flag)
    }
}

fn problem(param: &'static str, flag: &'static str, says: impl Into<String>) -> Problem {
    Problem { param, flag, says: says.into() }
}

/// A Google organisation id: digits, nothing else. Google's are twelve, but the
/// length is not a documented guarantee, so only the shape is judged.
pub(crate) fn organization_id(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("is empty".into());
    }
    if !value.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("is {:?}, which is not a number — an organisation id is all digits, and the DOMAIN is `customer_domain`", value));
    }
    Ok(())
}

/// A billing account id: `XXXXXX-XXXXXX-XXXXXX`, upper-case hex-ish groups.
pub(crate) fn billing_account(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("is empty".into());
    }
    let groups: Vec<&str> = value.split('-').collect();
    let shaped = groups.len() == 3
        && groups.iter().all(|g| g.len() == 6 && g.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
    if !shaped {
        return Err(format!("is {:?}, which is not a billing account id — the shape is XXXXXX-XXXXXX-XXXXXX", value));
    }
    Ok(())
}

/// A project id, by Google's rule: 6–30 characters, lower-case letter first,
/// then letters, digits and hyphens, and no trailing hyphen.
pub(crate) fn project_id(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("is empty".into());
    }
    let ok = (6..=30).contains(&value.len())
        && value.starts_with(|c: char| c.is_ascii_lowercase())
        && !value.ends_with('-')
        && value.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if !ok {
        return Err(format!(
            "is {:?}, which is not a project id — 6 to 30 characters, starting with a lower-case letter, then letters, digits and hyphens",
            value
        ));
    }
    Ok(())
}

/// A bucket name, by the rule that matters here: 3–63 characters, lower-case
/// letters, digits, hyphens, underscores and dots, starting and ending
/// alphanumeric. (Google's full rule has more, and the API enforces the rest.)
pub(crate) fn bucket_name(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("is empty".into());
    }
    let alnum = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let ok = (3..=63).contains(&value.len())
        && value.starts_with(alnum)
        && value.ends_with(alnum)
        && value.chars().all(|c| alnum(c) || c == '-' || c == '_' || c == '.');
    if !ok {
        return Err(format!(
            "is {:?}, which is not a bucket name — 3 to 63 characters of lower-case letters, digits, `-`, `_` and `.`, starting and ending with a letter or digit",
            value
        ));
    }
    Ok(())
}

/// What bootstrap is about to use. Borrowed, so the caller passes what it read.
pub(crate) struct DayZero<'a> {
    pub shortname: &'a str,
    /// Empty is legal only on the greenfield path, which has no organisation yet.
    pub organization_id: &'a str,
    pub greenfield: bool,
    pub billing_account: &'a str,
    /// `None` when the estate does not set it: there is no default to take.
    pub project_id: Option<&'a str>,
    /// `None` when the estate does not set it: there is no default to take. The one
    /// default is `presets/estate-core.satz`'s, which an estate that uses the pack
    /// resolves before bootstrap reads the table.
    pub bucket_name: Option<&'a str>,
}

/// Everything wrong with it, in the order an operator would fix it.
pub(crate) fn check(d: &DayZero<'_>) -> Vec<Problem> {
    let mut out = Vec::new();
    if d.shortname.trim().is_empty() {
        out.push(problem("customer_shortname", "customer-shortname", "is empty"));
    }
    // greenfield has no organisation to name yet: that is the whole point of it
    if !d.greenfield {
        if let Err(why) = organization_id(d.organization_id) {
            out.push(problem("customer_organization_id", "customer-organization-id", why));
        }
    }
    if let Err(why) = billing_account(d.billing_account) {
        out.push(problem("billing_account_infra", "billing-account-infra", why));
    }
    if let Err(why) = d.project_id.map_or(Err("is not set".to_string()), project_id) {
        out.push(problem("infra_project_name", "infra-project-name", why));
    }
    if let Err(why) = d.bucket_name.map_or(Err("is not set".to_string()), bucket_name) {
        out.push(problem("infra_bucket_name", "infra-bucket-name", why));
    }
    out
}

/// The gate itself: the problems as one refusal, or `Ok` to go on.
pub(crate) fn gate(d: &DayZero<'_>) -> Result<(), String> {
    let problems = check(d);
    if problems.is_empty() {
        return Ok(());
    }
    let mut msg = format!(
        "the estate is not ready to bootstrap — {} param(s) are missing or malformed, and nothing was called:\n",
        problems.len()
    );
    for p in &problems {
        msg.push_str(&p.to_string());
        msg.push('\n');
    }
    msg.push_str("  (`satz init` merges these into the estate you already have; nothing else is touched.)");
    Err(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok() -> DayZero<'static> {
        DayZero {
            shortname: "acme",
            organization_id: "123456789012",
            greenfield: false,
            billing_account: "012345-6789AB-CDEF01",
            project_id: Some("acme-infra-001"),
            bucket_name: Some("acme-infra-001-state"),
        }
    }

    #[test]
    fn a_complete_estate_passes_and_says_nothing() {
        assert_eq!(check(&ok()), Vec::new());
        assert!(gate(&ok()).is_ok());
    }

    #[test]
    fn an_empty_billing_account_is_caught_before_it_reaches_a_url() {
        // `billingAccounts/:testIamPermissions` came back as an HTML 404 page
        let d = DayZero { billing_account: "", ..ok() };
        let err = gate(&d).unwrap_err();
        assert!(err.contains("billing_account_infra — is empty"), "{}", err);
        assert!(err.contains("satz init --billing-account-infra"), "the flag that sets it:\n{}", err);
        assert!(err.contains("nothing was called"), "{}", err);
    }

    #[test]
    fn a_domain_in_the_organisation_id_is_named_for_what_it_is() {
        let d = DayZero { organization_id: "example.com", ..ok() };
        let err = gate(&d).unwrap_err();
        assert!(err.contains("not a number") && err.contains("customer_domain"), "{}", err);
    }

    #[test]
    fn greenfield_has_no_organisation_yet_and_that_is_allowed() {
        let d = DayZero { organization_id: "", greenfield: true, ..ok() };
        assert!(gate(&d).is_ok());
        // …but the rest is still judged
        let d = DayZero { organization_id: "", greenfield: true, billing_account: "nope", ..ok() };
        assert!(gate(&d).unwrap_err().contains("billing_account_infra"));
    }

    #[test]
    fn every_shape_is_judged_and_each_problem_is_reported_once() {
        let d = DayZero {
            shortname: "",
            organization_id: "12-34",
            greenfield: false,
            billing_account: "012345-6789AB",
            project_id: Some("Acme_Infra"),
            bucket_name: Some("-nope-"),
        };
        let problems = check(&d);
        assert_eq!(problems.len(), 5, "{:?}", problems);
        let params: Vec<&str> = problems.iter().map(|p| p.param).collect();
        assert_eq!(
            params,
            ["customer_shortname", "customer_organization_id", "billing_account_infra", "infra_project_name", "infra_bucket_name"]
        );
    }

    /// A project id the estate does not set is refused by name: bootstrap has no default
    /// for it, since the compile declares the project under the estate's own value.
    #[test]
    fn a_project_id_the_estate_does_not_set_is_refused_by_name() {
        let d = DayZero { project_id: None, bucket_name: None, ..ok() };
        let err = gate(&d).unwrap_err();
        assert!(err.contains("infra_project_name — is not set"), "{}", err);
        assert!(err.contains("satz init --infra-project-name"), "the flag that sets it:\n{}", err);
        assert!(err.contains("infra_bucket_name — is not set"), "{}", err);
    }

    /// The state bucket has one default, `presets/estate-core.satz`'s, and bootstrap does
    /// not hold a second: an estate that binds the project and not the bucket is refused
    /// by name rather than bootstrapped into a bucket named after the project.
    #[test]
    fn a_bucket_the_estate_does_not_set_is_refused_although_the_project_is_set() {
        let d = DayZero { bucket_name: None, ..ok() };
        let err = gate(&d).unwrap_err();
        assert!(err.contains("infra_bucket_name — is not set"), "{}", err);
        assert!(!err.contains("infra_project_name"), "the project is set and is not the bucket's default:\n{}", err);
    }

    #[test]
    fn the_shapes_themselves() {
        assert!(organization_id("123456789012").is_ok());
        assert!(organization_id("1").is_ok(), "length is not a documented guarantee");
        assert!(organization_id("organizations/123").is_err());
        assert!(billing_account("012345-6789AB-CDEF01").is_ok());
        assert!(billing_account("012345-6789ab-CDEF01").is_err(), "lower case is not how Google writes them");
        assert!(billing_account("012345-6789AB-CDEF01-X").is_err());
        assert!(project_id("acme-infra-001").is_ok());
        assert!(project_id("acme").is_err(), "under six characters");
        assert!(project_id("1acme-infra").is_err(), "must start with a letter");
        assert!(project_id("acme-infra-").is_err(), "no trailing hyphen");
        assert!(project_id("ACME-INFRA").is_err());
        assert!(bucket_name("acme-infra-001-state").is_ok());
        assert!(bucket_name("acme.infra_001").is_ok());
        assert!(bucket_name("ab").is_err());
        assert!(bucket_name("-acme").is_err());
    }
}
