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
