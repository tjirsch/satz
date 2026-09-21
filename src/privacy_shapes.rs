//! The privacy shapes, judged by satz: every token in a text that is SHAPED like
//! private data — a directory id, an organisation number, a billing account, a GUID,
//! an e-mail address, a domain, a project id, a repository URL — and is not one of
//! the documented example values.
//!
//! The rules are the content rules of `scripts/check-names.sh`, the repository's
//! privacy gate, in a second implementation (ADR 0050). The script stays the gate: it
//! runs where there is no toolchain and on a tree that does not compile. This copy is
//! what `satz review-pack` judges a pack with, because a pack written against its
//! author's own organisation carries exactly these shapes, and each one must become a
//! param before the pack goes upstream.
//!
//! Two copies, one proof: the allow-lists are ONE file both read
//! (`scripts/check-names-allow.txt`, compiled in below), and the corpus test at the
//! bottom runs the script and this module over the same fixtures
//! (`tests/privacy-shapes/`) and fails on any token one of them flags and the other
//! does not.
//!
//! Every rule judges TOKENS, never lines: an allowed value beside a private one never
//! shields it. The matching is ASCII, as the script's is under `LC_ALL=C`.

use std::sync::LazyLock;

use regex::bytes::{Regex, RegexBuilder};

/// The gate's allow-lists: `<list> <ERE>` per line, a list's entries joined into one
/// alternation — the file `scripts/check-names.sh` reads.
const ALLOW_FILE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/scripts/check-names-allow.txt"));

/// One kind of private-looking token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Shape {
    DirectoryId,
    Number,
    BillingAccount,
    Email,
    Domain,
    Guid,
    Guid32,
    ProjectPath,
    ProjectAssignment,
    RepositoryUrl,
}

impl Shape {
    /// In the order the gate reports them.
    #[cfg(test)]
    pub(crate) const ALL: &'static [Shape] = &[
        Shape::DirectoryId,
        Shape::Number,
        Shape::BillingAccount,
        Shape::Email,
        Shape::Domain,
        Shape::Guid,
        Shape::Guid32,
        Shape::ProjectPath,
        Shape::ProjectAssignment,
        Shape::RepositoryUrl,
    ];

    /// The rule's title exactly as `scripts/check-names.sh` prints it (`✗ <title>`):
    /// the corpus test reads the script's report by it.
    #[cfg(all(test, unix))]
    pub(crate) fn gate_rule(self) -> &'static str {
        match self {
            Shape::DirectoryId => "directory id (C0…) that is not an example value",
            Shape::Number => "11–13 digit number (org/project/folder id) that is not an example value",
            Shape::BillingAccount => "billing account id that is not an example value",
            Shape::Email => {
                "e-mail address outside reserved/vendor domains (placeholders like <customer-domain> are fine)"
            }
            Shape::Domain => "domain that is neither IANA-reserved nor a known vendor host (a real company's domain?)",
            Shape::Guid => "GUID that is neither an example value nor a documented vendor default (an Entra tenant id?)",
            Shape::Guid32 => "32 hex characters — an Entra tenant id without dashes is the workload identity pool id",
            Shape::ProjectPath => "project id that is not an example value (projects/…, project = …, --project)",
            Shape::ProjectAssignment => "project id in an assignment that is not an example value",
            Shape::RepositoryUrl => "customer repository URL or checkout path",
        }
    }

    /// What the token looks like it is, for a reader of a finding.
    pub(crate) fn describe(self) -> &'static str {
        match self {
            Shape::DirectoryId => "a Google Workspace directory id",
            Shape::Number => "an organisation, folder or project number",
            Shape::BillingAccount => "a billing account id",
            Shape::Email => "an e-mail address outside the reserved and vendor domains",
            Shape::Domain => "a domain that is neither IANA-reserved nor a known vendor host",
            Shape::Guid => "a GUID that is no documented vendor default — an Entra tenant id",
            Shape::Guid32 => "32 hex characters — an Entra tenant id without dashes, a workload identity pool id",
            Shape::ProjectPath => "a project id",
            Shape::ProjectAssignment => "a project id",
            Shape::RepositoryUrl => "a customer repository URL or checkout path",
        }
    }
}

/// One private-looking token, and the 1-based line it stands on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Hit {
    pub shape: Shape,
    pub line: u32,
    pub token: String,
}

/// A rule: the lines it reads, what it strips from them first, the tokens it takes
/// out, and the allow-list a token is held against (case-insensitive, like the
/// script's `grep -i -v`).
struct Rule {
    shape: Shape,
    /// a line the rule reads at all — before anything is stripped from it
    line: Option<Regex>,
    /// removed from the line before the tokens are taken
    strip: Vec<Regex>,
    token: Regex,
    allow: Option<Regex>,
}

fn re(pattern: &str) -> Regex {
    RegexBuilder::new(pattern)
        .unicode(false)
        .build()
        .unwrap_or_else(|e| panic!("privacy shape pattern {}: {}", pattern, e))
}

fn allow_re(pattern: String) -> Regex {
    RegexBuilder::new(&pattern)
        .unicode(false)
        .case_insensitive(true)
        .build()
        .unwrap_or_else(|e| panic!("privacy allow-list {}: {}", pattern, e))
}

/// One list of `scripts/check-names-allow.txt`, its entries joined with `|` — the
/// script's `allow()`. A list with no entry is the file broken, and nothing is judged
/// against nothing.
fn allow_list(name: &str) -> String {
    let entries: Vec<&str> =
        ALLOW_FILE.lines().filter_map(|l| l.strip_prefix(name).and_then(|rest| rest.strip_prefix(' '))).collect();
    assert!(!entries.is_empty(), "scripts/check-names-allow.txt has no '{}' entry", name);
    entries.join("|")
}

const GUID: &str = r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
const DOMAIN: &str = r"\b[a-z0-9-]+(\.[a-z0-9-]+)*\.(com|org|net|io|dev|de|eu|ch|at|uk|us|fr|it|nl|cloud|app|ai|co)\b";
const EMAIL: &str = r"[A-Za-z0-9._+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}";

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let project = allow_list("project");
    let domain = allow_list("domain");
    vec![
        Rule {
            shape: Shape::DirectoryId,
            line: None,
            strip: Vec::new(),
            token: re(r"\bC0[0-9a-z]{7}\b"),
            allow: Some(allow_re(format!(r"\b({})\b", allow_list("dir")))),
        },
        Rule {
            shape: Shape::Number,
            // a number inside a longer hex run or a GUID is part of that, not an id:
            // both are removed first, from a line that held a number to begin with
            line: Some(re(r"\b[0-9]{11,13}\b")),
            strip: vec![re(r"[0-9a-fA-F]{20,}"), re(GUID)],
            token: re(r"\b[0-9]{11,13}\b"),
            allow: Some(allow_re(format!(r"\b({})\b", allow_list("num")))),
        },
        Rule {
            shape: Shape::BillingAccount,
            line: None,
            strip: Vec::new(),
            token: re(r"\b[0-9A-F]{6}-[0-9A-F]{6}-[0-9A-F]{6}\b"),
            allow: Some(allow_re(format!("({})", allow_list("bill")))),
        },
        Rule {
            shape: Shape::Email,
            line: None,
            strip: Vec::new(),
            token: re(EMAIL),
            allow: Some(allow_re(format!(r"@({})\b", domain))),
        },
        Rule {
            shape: Shape::Domain,
            line: None,
            strip: Vec::new(),
            token: re(DOMAIN),
            allow: Some(allow_re(format!("^({})$", domain))),
        },
        Rule {
            shape: Shape::Guid,
            line: None,
            strip: Vec::new(),
            token: re(&format!(r"\b{}\b", GUID)),
            allow: Some(allow_re(format!("^({})$", allow_list("guid")))),
        },
        Rule {
            shape: Shape::Guid32,
            line: None,
            strip: Vec::new(),
            token: re(r"\b[0-9a-fA-F]{32}\b"),
            allow: Some(allow_re(format!("^({})$", allow_list("guid32")))),
        },
        Rule {
            shape: Shape::ProjectPath,
            line: None,
            strip: Vec::new(),
            token: re(r"projects/[a-z][a-z0-9-]{3,28}[a-z0-9]"),
            allow: Some(allow_re(format!("^projects/({})$", project))),
        },
        Rule {
            shape: Shape::ProjectAssignment,
            line: None,
            strip: Vec::new(),
            token: re(r#"project(_id)?[[:space:]]*=[[:space:]]*"[a-z][a-z0-9-]{4,28}[a-z0-9]""#),
            allow: Some(allow_re(format!(r#"=[[:space:]]*"({})"$"#, project))),
        },
        Rule {
            shape: Shape::RepositoryUrl,
            line: None,
            strip: Vec::new(),
            // the longer alternative first: the script's grep takes the longest match
            // at a position, this engine the first alternative that matches
            token: re(r"source\.developers\.google\.com|~/projects/([a-z]+/[a-z]+-C0|organizations)"),
            allow: None,
        },
    ]
});

/// Every private-looking token of `text` that is not an allowed value, by rule and
/// then by line — the order the gate reports them in.
pub(crate) fn scan(text: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    for rule in RULES.iter() {
        for (i, line) in text.as_bytes().split(|b| *b == b'\n').enumerate() {
            if rule.line.as_ref().is_some_and(|l| !l.is_match(line)) {
                continue;
            }
            let mut owned = line.to_vec();
            for s in &rule.strip {
                owned = s.replace_all(&owned, &b""[..]).into_owned();
            }
            for m in rule.token.find_iter(&owned) {
                let token = m.as_bytes();
                if rule.allow.as_ref().is_some_and(|a| a.is_match(token)) {
                    continue;
                }
                hits.push(Hit { shape: rule.shape, line: i as u32 + 1, token: String::from_utf8_lossy(token).into_owned() });
            }
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    /// The corpus files are committed with every private-looking value split by this
    /// marker, so the repository's own gate — which reads every tracked file — finds no
    /// shape in them. The test removes it before either implementation reads the text.
    const JOIN: &str = "%%";

    fn corpus_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/privacy-shapes")
    }

    #[cfg(unix)]
    fn corpus() -> Vec<(String, String)> {
        let mut files: Vec<(String, String)> = std::fs::read_dir(corpus_dir())
            .expect("tests/privacy-shapes is there")
            .map(|e| e.unwrap().path())
            .filter(|p| p.is_file())
            .map(|p| {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                (name, std::fs::read_to_string(&p).unwrap().replace(JOIN, ""))
            })
            .collect();
        files.sort();
        assert!(files.len() >= 10, "the corpus is {} files", files.len());
        files
    }

    #[cfg(unix)]
    fn shape_of_rule(title: &str) -> Shape {
        Shape::ALL
            .iter()
            .copied()
            .find(|s| s.gate_rule() == title)
            .unwrap_or_else(|| panic!("the script reports a rule satz does not know: {:?}", title))
    }

    /// What `scripts/check-names.sh FILE` flags in one file, read out of its report:
    /// `✗ <rule>` above one `    <file>:<line>: <token>` row per token.
    #[cfg(unix)]
    fn gate_hits(file: &Path) -> (bool, BTreeSet<Hit>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let out = std::process::Command::new("bash")
            .arg("scripts/check-names.sh")
            .arg(file)
            .current_dir(root)
            .env("LC_ALL", "C")
            .env("NAMES_DENYLIST", "/nonexistent/denylist.txt")
            .output()
            .expect("bash runs the gate");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr);
        let mut hits = BTreeSet::new();
        let mut rule: Option<Shape> = None;
        for l in text.lines() {
            if let Some(title) = l.strip_prefix("✗ ") {
                rule = Some(shape_of_rule(title));
            } else if let Some(row) = l.strip_prefix("    ") {
                let shape = rule.unwrap_or_else(|| panic!("a row under no rule: {:?}\n{}", l, text));
                let (_, rest) = row.split_once(':').unwrap_or_else(|| panic!("not file:line: token: {:?}", row));
                let (line, token) = rest.split_once(": ").unwrap_or_else(|| panic!("not file:line: token: {:?}", row));
                hits.insert(Hit { shape, line: line.parse().unwrap(), token: token.to_string() });
            } else {
                assert!(l.is_empty() || l.starts_with("check-names: "), "a line the report does not have: {:?}\n{}", l, text);
            }
        }
        let failed = !out.status.success();
        assert!(
            text.contains("check-names: OK") || text.contains("check-names: FAILED"),
            "the gate did not finish: {}\n{}",
            text,
            stderr
        );
        (failed, hits)
    }

    /// Two copies, one proof: every corpus file goes through the gate script and through
    /// this module, and the two must flag the same tokens at the same lines under the
    /// same rule — and pass or fail the file alike. A change to either side alone that
    /// moves any verdict in the corpus fails here. A `clean-` file must pass, a `hit-`
    /// file must fail, so the corpus cannot quietly stop exercising a rule.
    #[cfg(unix)]
    #[test]
    fn the_gate_script_and_satz_flag_the_same_tokens_in_every_corpus_file() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let scratch = root.join("target").join(format!("privacy-shapes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let mut rules_seen: BTreeSet<Shape> = BTreeSet::new();
        for (name, text) in corpus() {
            let file = scratch.join(&name);
            std::fs::write(&file, &text).unwrap();
            let (gate_failed, gate) = gate_hits(&file);
            let satz: BTreeSet<Hit> = scan(&text).into_iter().collect();
            assert_eq!(gate, satz, "{}: the gate (left) and satz (right) disagree", name);
            assert_eq!(gate_failed, !satz.is_empty(), "{}: the gate's exit status and its rows disagree", name);
            if name.starts_with("clean-") {
                assert!(satz.is_empty(), "{}: a clean fixture flags {:?}", name, satz);
            } else {
                assert!(name.starts_with("hit-"), "{}: a fixture is `clean-…` or `hit-…`", name);
                assert!(!satz.is_empty(), "{}: a hit fixture flags nothing", name);
            }
            rules_seen.extend(satz.iter().map(|h| h.shape));
        }
        let _ = std::fs::remove_dir_all(&scratch);
        let unexercised: Vec<Shape> = Shape::ALL.iter().copied().filter(|s| !rules_seen.contains(s)).collect();
        assert!(unexercised.is_empty(), "no corpus file exercises {:?}", unexercised);
    }

    /// The committed corpus carries no shape: the repository's gate reads every tracked
    /// file, so a fixture with an unsplit value would stop every commit.
    #[test]
    fn the_committed_corpus_is_clean_until_its_markers_are_joined() {
        for entry in std::fs::read_dir(corpus_dir()).unwrap() {
            let path = entry.unwrap().path();
            let raw = std::fs::read_to_string(&path).unwrap();
            assert_eq!(scan(&raw), Vec::new(), "{} carries a shape before its markers are joined", path.display());
        }
    }

    /// Every list the script reads is in the file, and every entry compiles here too.
    #[test]
    fn every_allow_list_is_in_the_one_file() {
        for name in ["dir", "num", "bill", "domain", "guid", "guid32", "project"] {
            assert!(!allow_list(name).is_empty());
        }
        assert_eq!(RULES.len(), Shape::ALL.len());
    }

    /// The library ships through the gate, so every pack satz ships scans clean here.
    #[test]
    fn every_shipped_pack_scans_clean() {
        let presets = Path::new(env!("CARGO_MANIFEST_DIR")).join("presets");
        let mut stack = vec![presets];
        let mut n = 0;
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let p = entry.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|e| e == "satz") {
                    let hits = scan(&std::fs::read_to_string(&p).unwrap());
                    assert!(hits.is_empty(), "{}: {:?}", p.display(), hits);
                    n += 1;
                }
            }
        }
        assert!(n > 10, "{} packs", n);
    }

    /// Per token, never per line: an allowed value beside a private one does not shield it.
    #[test]
    fn an_allowed_value_on_the_line_shields_nothing() {
        let private = format!("{}{}", "98765", "4321098");
        let text = format!("org = \"123456789012\" other = \"{}\"\n", private);
        assert_eq!(scan(&text), vec![Hit { shape: Shape::Number, line: 1, token: private }]);
    }
}
