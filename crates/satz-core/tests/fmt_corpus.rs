//! The formatter against every Satz file in the repository: formatting is
//! idempotent and changes nothing the compiler sees. `check-presets` compares the
//! canonical form of the parsed file, so canonical equality is the exact
//! guarantee that a reformatted pack is not drift.

use satz_core::fmt::format;
use satz_core::satz::{canonical, canonical_questions, parse};
use std::path::{Path, PathBuf};

/// The repository's Satz files are the TRACKED ones: the smoke matrix writes
/// `.satz` files of its own under `tests/smoke/` (imports, skeletons), and those
/// are its business, not this test's.
fn tracked_satz_files(root: &Path) -> Vec<PathBuf> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "-z", "--", "presets", "tests"])
        .current_dir(root)
        .output()
        .expect("git ls-files: the corpus is defined as the tracked files, so git must run");
    assert!(out.status.success(), "git ls-files failed: {}", String::from_utf8_lossy(&out.stderr));
    let mut files: Vec<PathBuf> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|f| f.ends_with(".satz") && !f.ends_with(".diff.satz"))
        .map(|f| root.join(f))
        .collect();
    files.sort();
    files
}

#[test]
fn every_corpus_file_formats_idempotently_and_keeps_its_meaning() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let files = tracked_satz_files(&root);
    assert!(files.len() > 50, "the corpus went missing: {} files", files.len());

    let mut unformatted = Vec::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let name = f.strip_prefix(&root).unwrap().display().to_string();
        let once = format(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
        let twice = format(&once).unwrap_or_else(|e| panic!("{name} (second pass): {e}"));
        assert_eq!(once, twice, "{name}: formatting is not idempotent");
        let before = parse(&src).unwrap();
        let after = parse(&once).unwrap_or_else(|e| panic!("{name}: formatted output does not parse: {e}"));
        assert_eq!(canonical(&before), canonical(&after), "{name}: formatting changed the canonical form");
        assert_eq!(canonical_questions(&before), canonical_questions(&after), "{name}: formatting changed the questions");
        if once != src {
            unformatted.push(name);
        }
    }
    // Every file in the repository is formatted; `satz fmt --check` in the smoke
    // matrix says the same from the outside.
    assert!(unformatted.is_empty(), "not formatted (run `satz fmt` on them): {unformatted:#?}");
}

/// A Windows checkout with CRLF line endings is the same Satz: the same canonical form,
/// the same questions, the same formatted text, and formatted exactly when its LF twin
/// is. For every tracked file.
#[test]
fn every_corpus_file_reads_the_same_with_crlf_line_endings() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    for f in tracked_satz_files(&root) {
        let src = std::fs::read_to_string(&f).unwrap();
        let crlf = src.replace('\n', "\r\n");
        let name = f.strip_prefix(&root).unwrap().display().to_string();
        let (a, b) = (parse(&src).unwrap(), parse(&crlf).unwrap_or_else(|e| panic!("{name} with CRLF: {e}")));
        assert_eq!(canonical(&a), canonical(&b), "{name}: CRLF changed the canonical form");
        assert_eq!(canonical_questions(&a), canonical_questions(&b), "{name}: CRLF changed the questions");
        assert_eq!(format(&src).unwrap(), format(&crlf).unwrap(), "{name}: CRLF formats differently");
        assert_eq!(
            satz_core::fmt::is_formatted(&src).unwrap(),
            satz_core::fmt::is_formatted(&crlf).unwrap(),
            "{name}: CRLF changed whether the file counts as formatted"
        );
    }
}

/// The two places a `\r` used to survive into the emission.
#[test]
fn a_crlf_heredoc_and_hcl_body_carry_no_carriage_return() {
    let lf = "estate e\n\nparams {\n  note = \"\"\"\n    one\n    two\n  \"\"\"\n}\n\nhcl trust \"reviewed\" {\n  output \"x\" {\n    value = 1\n  }\n}\n";
    let crlf = lf.replace('\n', "\r\n");
    let (a, b) = (parse(lf).unwrap(), parse(&crlf).unwrap());
    assert_eq!(canonical(&a), canonical(&b));
    assert!(!canonical(&b).contains('\r'), "{}", canonical(&b));
}
