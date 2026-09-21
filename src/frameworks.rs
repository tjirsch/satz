//! `compliance_frameworks` — the catalogs an estate is HELD TO.
//!
//! What an estate CLAIMS comes from its packs: a pack declares `claim cis-gcp 5.0 …`
//! and the compliance plane folds those claims into a goal view. What its customer
//! ANSWERS TO is a different fact — a contract, an auditor, a regulator — and no pack
//! can know it. An estate can claim CIS controls while its customer is audited against
//! ISO 27001.
//!
//! So the estate states it, as a day-0 param in `presets/estate-core.satz` with a
//! question beside it, and the commands whose output an auditor reads take it from
//! there: `report-compliance` with no framework reports each one, `satz prowler` scans
//! for them, and a pack reads the param like any other.
//!
//! A value has to name a catalog in `<presets_dir>/catalogs/`. A value that names none
//! is refused at compile time, with the list of catalogs that exist — a typo here is a
//! report that silently covers a framework nobody is held to.

use satz_core::pipeline::Env;
use std::path::Path;

/// The param the estate binds. One name, read by everything.
pub(crate) const PARAM: &str = "compliance_frameworks";

/// One framework the estate is held to: the catalog id it is named by, and the
/// `catalog`/`version` pair inside that catalog file — which is what a claim, a
/// cross-walk key and Prowler's framework list are keyed by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Framework {
    /// the catalog id, as the param writes it and as `require` takes it: `cis-gcp-5.0`
    pub id: String,
    /// the catalog's own name: `cis-gcp`
    pub catalog: String,
    /// the catalog's own version: `5.0`
    pub version: String,
}

/// The values the estate binds, or nothing when it binds no such param.
///
/// The shape is a list of strings. Anything else is an error rather than a coerced
/// value: a single string would read as one framework and a number as none.
pub(crate) fn declared(env: &Env) -> Result<Option<Vec<String>>, String> {
    let Some(v) = env.get(PARAM) else { return Ok(None) };
    let serde_yaml::Value::Sequence(items) = v else {
        return Err(format!("`{}` is a list of catalog ids, e.g. [\"cis-gcp-5.0\"]", PARAM));
    };
    let mut out = Vec::new();
    for item in items {
        match item.as_str() {
            Some(s) => out.push(s.to_string()),
            None => {
                return Err(format!("`{}`: every entry is a catalog id in quotes, e.g. [\"cis-gcp-5.0\"]", PARAM))
            }
        }
    }
    Ok(Some(out))
}

/// Every catalog the library holds, by id, sorted — what an error lists.
pub(crate) fn available(presets_dir: &str) -> Vec<String> {
    let dir = Path::new(presets_dir).join("catalogs");
    let Ok(entries) = std::fs::read_dir(&dir) else { return Vec::new() };
    let mut out: Vec<String> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))
        .filter_map(|e| e.path().file_stem().and_then(|s| s.to_str()).map(str::to_string))
        .collect();
    out.sort();
    out
}

/// The frameworks the estate is held to, each resolved against its catalog.
///
/// `Ok(None)` is an estate that states nothing — a different thing from an estate that
/// states an empty list, which is an estate held to nothing and reports nothing.
/// A value that names no catalog, or one that cannot be read, is an error naming the
/// value and listing the catalogs that exist: a framework that is listed and skipped is
/// how an evidence report starts lying about its own scope.
pub(crate) fn resolve(env: &Env, presets_dir: &str) -> Result<Option<Vec<Framework>>, String> {
    let Some(ids) = declared(env)? else { return Ok(None) };
    let mut out = Vec::new();
    for id in ids {
        out.push(one(&id, presets_dir)?);
    }
    Ok(Some(out))
}

/// One catalog id, resolved — the same error whether it is missing or unreadable.
pub(crate) fn one(id: &str, presets_dir: &str) -> Result<Framework, String> {
    let path = Path::new(presets_dir).join("catalogs").join(format!("{}.yaml", id));
    let known = available(presets_dir);
    let listed =
        if known.is_empty() { format!("none in {}", Path::new(presets_dir).join("catalogs").display()) } else { known.join(", ") };
    if !known.iter().any(|k| k == id) {
        return Err(format!("`{}`: {} names no catalog. Catalogs: {}", PARAM, id, listed));
    }
    // Listed, and the file is there — anything that stops it being READ is an error of
    // its own, never this framework quietly leaving the report.
    let text = crate::fsx::read_to_string(&path)
        .map_err(|e| format!("`{}`: catalog {} cannot be read: {}", PARAM, path.display(), e))?;
    #[derive(serde::Deserialize)]
    struct Header {
        catalog: String,
        version: String,
    }
    let h: Header = serde_yaml::from_str(&text)
        .map_err(|e| format!("`{}`: catalog {} does not parse: {}", PARAM, path.display(), e))?;
    Ok(Framework { id: id.to_string(), catalog: h.catalog, version: h.version })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(v: serde_yaml::Value) -> Env {
        Env::from([(PARAM.to_string(), v)])
    }

    fn catalogs(dir: &Path) {
        let c = dir.join("catalogs");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(c.join("cis-gcp-5.0.yaml"), "catalog: cis-gcp\nversion: \"5.0\"\ncontrols: {}\n").unwrap();
        std::fs::write(c.join("iso27001-2022.yaml"), "catalog: iso27001\nversion: \"2022\"\ncontrols: {}\n").unwrap();
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("satz-frameworks-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_estate_that_states_nothing_is_not_an_estate_held_to_nothing() {
        // No param at all: the caller decides what that means (report-compliance asks
        // for a framework). An EMPTY list is the estate saying "held to nothing", which
        // is an answer and resolves to no frameworks.
        assert_eq!(declared(&Env::new()).unwrap(), None);
        assert_eq!(declared(&env(serde_yaml::Value::Sequence(vec![]))).unwrap(), Some(vec![]));
    }

    #[test]
    fn the_shape_is_a_list_of_ids_and_nothing_else() {
        let e = declared(&env(serde_yaml::Value::String("cis-gcp-5.0".into()))).unwrap_err();
        assert!(e.contains("a list of catalog ids"), "{e}");
        let e = declared(&env(serde_yaml::Value::Sequence(vec![serde_yaml::Value::Number(5.into())]))).unwrap_err();
        assert!(e.contains("catalog id in quotes"), "{e}");
    }

    #[test]
    fn each_value_resolves_to_its_catalogs_own_name_and_version() {
        let dir = scratch("resolve");
        catalogs(&dir);
        let ids = serde_yaml::Value::Sequence(vec!["cis-gcp-5.0".into(), "iso27001-2022".into()]);
        let got = resolve(&env(ids), dir.to_str().unwrap()).unwrap().unwrap();
        assert_eq!(got[0], Framework { id: "cis-gcp-5.0".into(), catalog: "cis-gcp".into(), version: "5.0".into() });
        assert_eq!(got[1], Framework { id: "iso27001-2022".into(), catalog: "iso27001".into(), version: "2022".into() });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_value_naming_no_catalog_is_refused_with_the_list() {
        // The typo case, and the whole reason the value is checked at all: a report
        // that skipped the framework would read as a clean one.
        let dir = scratch("typo");
        catalogs(&dir);
        let ids = serde_yaml::Value::Sequence(vec!["cis-gcp-6.0".into()]);
        let e = resolve(&env(ids), dir.to_str().unwrap()).unwrap_err();
        assert_eq!(e, "`compliance_frameworks`: cis-gcp-6.0 names no catalog. Catalogs: cis-gcp-5.0, iso27001-2022");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_catalog_that_is_there_and_cannot_be_read_is_an_error_of_its_own() {
        // Not the same fault as a typo, and not a framework that quietly leaves the
        // report: an evidence report that skipped a listed framework would read clean.
        let dir = scratch("unreadable");
        catalogs(&dir);
        std::fs::write(dir.join("catalogs/cis-gcp-5.0.yaml"), "catalog: [not a string\n").unwrap();
        let ids = serde_yaml::Value::Sequence(vec!["cis-gcp-5.0".into()]);
        let e = resolve(&env(ids), dir.to_str().unwrap()).unwrap_err();
        assert!(e.contains("does not parse"), "{e}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
