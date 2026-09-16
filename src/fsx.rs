//! Filesystem helpers that always include the failing path and the OS reason in
//! their error messages.
//!
//! Plain `std::fs` returns an `io::Error` whose `Display` is just the OS reason
//! (e.g. `No such file or directory (os error 2)`) with no indication of *which*
//! path failed. These thin wrappers preserve the original `ErrorKind` and wrap the
//! message with the action and the path, so a failure reads like:
//!
//! ```text
//! failed to read file 'config.toml': No such file or directory (os error 2)
//! ```
//!
//! They return `io::Result<T>`, so they are drop-in replacements at call sites that
//! propagate into `Box<dyn std::error::Error>` via `?`.
//!
//! Satz text has three ways to disk and no fourth: [`write_generated_satz`] for a
//! file satz composes whole, [`write_edited_satz`] for a splice into a file its
//! author wrote, [`write_verbatim`] for bytes satz copies rather than composes.
//! [`write`] refuses a `.satz` path, so every Satz file satz writes is in the
//! canonical layout or deliberately not — never by accident. `clippy.toml`
//! disallows `std::fs::write` outside this module, so no non-test code writes a
//! file except through it.

use std::fs::{DirEntry, File, Permissions};
use std::io;
use std::path::Path;

/// Wrap an `io::Error` with the attempted action and path while keeping its kind.
fn ctx(action: &str, path: &Path, e: io::Error) -> io::Error {
    io::Error::new(
        e.kind(),
        format!("failed to {action} '{}': {e}", path.display()),
    )
}

pub fn read_to_string<P: AsRef<Path>>(path: P) -> io::Result<String> {
    let path = path.as_ref();
    std::fs::read_to_string(path).map_err(|e| ctx("read file", path, e))
}

/// A Satz source file: `.satz`, but not the `.diff.satz` a merge writes, which is
/// a unified diff.
fn is_satz_source(path: &Path) -> bool {
    let name = path.to_string_lossy();
    name.ends_with(".satz") && !name.ends_with(".diff.satz")
}

/// The one call to `std::fs::write` in non-test code: `clippy.toml` disallows it
/// everywhere else, so every file satz writes goes through this module.
#[allow(clippy::disallowed_methods)]
fn write_bytes(path: &Path, contents: &[u8]) -> io::Result<()> {
    std::fs::write(path, contents).map_err(|e| ctx("write file", path, e))
}

/// Write any file that is not Satz source. A `.satz` path is refused: Satz text
/// goes through `write_generated_satz`, `write_edited_satz` or `write_verbatim`,
/// each saying what it does to the layout.
pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    let path = path.as_ref();
    if is_satz_source(path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "refusing to write '{}' through fsx::write: Satz text is written with \
                 write_generated_satz (composed whole, formatted), write_edited_satz (a splice, \
                 the author's layout kept) or write_verbatim (bytes satz copies)",
                path.display()
            ),
        ));
    }
    write_bytes(path, contents.as_ref())
}

/// A Satz file satz composes whole — `init`, a skeleton, `import`,
/// `export-organizational-policies`: written in the canonical layout, always.
/// Text the parser refuses is written as it is and the error returned: the file
/// is there to look at, and the generator that produced it is a defect, never
/// healed.
pub fn write_generated_satz<P: AsRef<Path>>(path: P, text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    match satz_core::fmt::format(text) {
        Ok(formatted) => Ok(write_bytes(path, formatted.as_bytes())?),
        Err(e) => {
            write_bytes(path, text.as_bytes())?;
            Err(format!("{}: satz wrote a file it cannot parse ({}) — a generator defect, the file is there to look at", path.display(), e).into())
        }
    }
}

/// A splice into a file its author wrote — an answer, an import-id, a role, a
/// `use` line. The author's layout stays; a file that was formatted before is
/// formatted after. Text the parser refuses is not written: the splice is the
/// defect, and the file the author had is still there.
pub fn write_edited_satz<P: AsRef<Path>>(path: P, before: &str, after: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = path.as_ref();
    let formatted = satz_core::fmt::format(after).map_err(|e| {
        format!("{}: the edit would leave a file satz cannot parse ({}) — nothing written", path.display(), e)
    })?;
    let was_formatted = satz_core::fmt::is_formatted(before)
        .map_err(|e| format!("{}: the file did not parse before the edit ({})", path.display(), e))?;
    let out = if was_formatted { formatted.as_bytes() } else { after.as_bytes() };
    Ok(write_bytes(path, out)?)
}

/// Bytes satz copies rather than composes — a pristine pack from upstream, the
/// fork that keeps an author's old file, a restore after a failed edit, what
/// `satz fmt` itself produces. Written as given, whatever the path.
pub fn write_verbatim<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    write_bytes(path.as_ref(), contents.as_ref())
}

pub fn create_dir_all<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let path = path.as_ref();
    std::fs::create_dir_all(path).map_err(|e| ctx("create directory", path, e))
}

pub fn remove_file<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let path = path.as_ref();
    std::fs::remove_file(path).map_err(|e| ctx("delete file", path, e))
}

pub fn remove_dir_all<P: AsRef<Path>>(path: P) -> io::Result<()> {
    let path = path.as_ref();
    std::fs::remove_dir_all(path).map_err(|e| ctx("remove directory", path, e))
}

pub fn create_file<P: AsRef<Path>>(path: P) -> io::Result<File> {
    let path = path.as_ref();
    File::create(path).map_err(|e| ctx("create file", path, e))
}

/// Unix only: every caller sets an executable bit, which Windows does not have.
#[cfg(unix)]
pub fn set_permissions<P: AsRef<Path>>(path: P, perm: Permissions) -> io::Result<()> {
    let path = path.as_ref();
    std::fs::set_permissions(path, perm).map_err(|e| ctx("set permissions on", path, e))
}

/// Read a directory and collect its entries, annotating both the `read_dir` call
/// and any per-entry error with the directory path.
pub fn read_dir_entries<P: AsRef<Path>>(path: P) -> io::Result<Vec<DirEntry>> {
    let path = path.as_ref();
    let entries = std::fs::read_dir(path).map_err(|e| ctx("read directory", path, e))?;
    let mut out = Vec::new();
    for entry in entries {
        out.push(entry.map_err(|e| ctx("read directory entry in", path, e))?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("satz-fsx-{}-{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const MESSY: &str = "estate x\nparams {\na = 1\n  long_name=2\n}\n";
    const CANON: &str = "estate x\nparams {\n  a         = 1\n  long_name = 2\n}\n";

    #[test]
    fn generated_satz_lands_formatted() {
        let p = scratch("gen").join("e.satz");
        write_generated_satz(&p, MESSY).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), CANON);
    }

    #[test]
    fn generated_satz_that_does_not_parse_is_written_and_reported() {
        let p = scratch("gen-bad").join("e.satz");
        let e = write_generated_satz(&p, "a = \"open\n").unwrap_err().to_string();
        assert!(e.contains("cannot parse") && e.contains("generator defect"), "{e}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "a = \"open\n");
    }

    #[test]
    fn an_edit_of_a_formatted_file_stays_formatted() {
        let p = scratch("edit-fmt").join("e.satz");
        let after = "estate x\nparams {\n  a         = 1\n  long_name = 2\n  customer_domain = \"example.com\"\n}\n";
        write_edited_satz(&p, CANON, after).unwrap();
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            "estate x\nparams {\n  a               = 1\n  long_name       = 2\n  customer_domain = \"example.com\"\n}\n"
        );
    }

    #[test]
    fn an_edit_of_an_unformatted_file_keeps_its_layout() {
        let p = scratch("edit-raw").join("e.satz");
        let after = "estate x\nparams {\na = 1\n  long_name=2\n  b = 3\n}\n";
        write_edited_satz(&p, MESSY, after).unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), after);
    }

    #[test]
    fn an_edit_that_breaks_the_file_writes_nothing() {
        let p = scratch("edit-bad").join("e.satz");
        std::fs::write(&p, CANON).unwrap();
        let e = write_edited_satz(&p, CANON, "estate x\nparams {\n  a = \"open\n}\n").unwrap_err().to_string();
        assert!(e.contains("nothing written"), "{e}");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), CANON);
    }

    #[test]
    fn write_refuses_satz_source_and_accepts_the_rest() {
        let dir = scratch("refuse");
        let e = write(dir.join("x.satz"), "estate x\n").unwrap_err().to_string();
        assert!(e.contains("write_generated_satz"), "{e}");
        write(dir.join("x.diff.satz"), "@@ -1 +1 @@\n").unwrap();
        write(dir.join("x.tf"), "resource {}\n").unwrap();
        write_verbatim(dir.join("y.satz"), "estate y\n").unwrap();
    }

}
