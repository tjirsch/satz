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

pub fn write<P: AsRef<Path>, C: AsRef<[u8]>>(path: P, contents: C) -> io::Result<()> {
    let path = path.as_ref();
    std::fs::write(path, contents).map_err(|e| ctx("write file", path, e))
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
