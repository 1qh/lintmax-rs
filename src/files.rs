//! Version-control-aware file enumeration shared by every tree scan.

use alloc::collections::BTreeSet;
use std::{
    ffi::OsString,
    fs::{self, DirEntry},
    io,
    path::{Path, PathBuf},
    process::Command,
};

use crate::discard;

/// Walks a root, retaining tracked and untracked-but-not-ignored files in Git.
/// Outside a checkout every file counts; directory symlinks are not followed.
///
/// # Errors
/// Returns an error if the root, checkout listing or directory entries cannot be read.
#[inline]
pub fn files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let directory = fs::canonicalize(root)?;
    let allowed = allowed_paths(&directory)?;
    let mut found = Vec::new();
    walk(&directory, allowed.as_ref(), &mut found)?;
    found.sort();
    Ok(found)
}

/// Restricts a checkout walk to the version-control candidate set.
///
/// # Errors
/// Returns an error if checkout metadata or the Git listing cannot be read.
fn allowed_paths(root: &Path) -> io::Result<Option<BTreeSet<PathBuf>>> {
    let checkout = has_checkout(root)?;
    if checkout {
        return git_paths(root).map(Some);
    }
    Ok(None)
}

/// Checks whether the root lies inside a checkout.
///
/// # Errors
/// Returns an error if checkout metadata cannot be inspected.
fn has_checkout(root: &Path) -> io::Result<bool> {
    for ancestor in root.ancestors() {
        match fs::symlink_metadata(ancestor.join(".git")) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// Reads the checkout's tracked and untracked-but-not-ignored paths.
///
/// # Errors
/// Returns an error if Git fails or a pathname cannot be decoded.
fn git_paths(root: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "git ls-files failed in {}: {}",
            root.display(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let mut paths = BTreeSet::new();
    let names = output.stdout.split(|byte| *byte == 0);
    for bytes in names.filter(|name| !name.is_empty()) {
        #[cfg(unix)]
        let path = listing_path(bytes);
        #[cfg(not(unix))]
        let path = match listing_path(bytes) {
            Ok(decoded) => decoded,
            Err(error) => return Err(error),
        };
        discard(paths.insert(root.join(path)));
    }
    Ok(paths)
}

/// Decodes a NUL-delimited Git pathname without losing Unix filename bytes.
#[cfg(unix)]
fn listing_path(bytes: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStringExt as _;
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

/// Decodes a Git pathname on platforms requiring Unicode filenames.
///
/// # Errors
/// Returns an error if the pathname is not valid UTF-8.
#[cfg(not(unix))]
fn listing_path(bytes: &[u8]) -> io::Result<PathBuf> {
    return String::from_utf8(bytes.to_vec())
        .map(|path| return PathBuf::from(OsString::from(path)))
        .map_err(|error| return io::Error::new(io::ErrorKind::InvalidData, error));
}

/// Returns eligible files, preserving enumeration failures for the caller.
///
/// # Errors
/// Returns an error if the root, checkout listing or directory entries cannot be read.
#[inline]
pub fn required(root: &Path) -> io::Result<Vec<PathBuf>> {
    files(root)
}

/// Recurses through real directories, pruning those with no eligible paths.
///
/// # Errors
/// Returns an error if a directory or its entries cannot be read.
fn walk(
    root: &Path,
    allowed: Option<&BTreeSet<PathBuf>>,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let entries = fs::read_dir(root)?;
    for result in entries {
        walk_entry(result, allowed, found)?;
    }
    Ok(())
}

/// Reads and visits one directory entry.
///
/// # Errors
/// Returns an error if the entry or its descendants cannot be read.
fn walk_entry(
    result: io::Result<DirEntry>,
    allowed: Option<&BTreeSet<PathBuf>>,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let entry = result?;
    visit(&entry, allowed, found)
}

/// Visits one entry without following directory symlinks.
///
/// # Errors
/// Returns an error if entry metadata or a descendant cannot be read.
fn visit(
    entry: &DirEntry,
    allowed: Option<&BTreeSet<PathBuf>>,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    if entry.file_name() == ".git" {
        return Ok(());
    }
    let kind = entry.file_type()?;
    let path = entry.path();
    if kind.is_dir() {
        return walk_allowed(&path, allowed, found);
    }
    if !path.is_dir() && allowed.is_none_or(|paths| paths.contains(&path)) {
        found.push(path);
    }
    Ok(())
}

/// Prunes directories containing no eligible paths before recursing.
///
/// # Errors
/// Returns an error if an eligible directory or its entries cannot be read.
fn walk_allowed(
    path: &Path,
    allowed: Option<&BTreeSet<PathBuf>>,
    found: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let eligible = allowed.is_none_or(|paths| paths.iter().any(|file| file.starts_with(path)));
    if eligible {
        return walk(path, allowed, found);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
