use core::sync::atomic::{AtomicU64, Ordering};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{self, Command},
};

use super::{files, required};

/// Unique suffix for temporary fixture directories within this process.
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

/// A temporary fixture removed even if an assertion unwinds.
struct Fixture {
    /// Root of the fixture tree.
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        crate::discard(fs::remove_dir_all(&self.root));
    }
}

/// Creates a fixture outside the source checkout without changing cwd.
fn fixture() -> Fixture {
    let root = env::temp_dir().join(format!(
        "lintmax-files-{}-{}",
        process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    drop(fs::create_dir_all(&root));
    return Fixture {
        root: fs::canonicalize(&root).unwrap_or(root),
    };
}

/// Runs Git only when the test executes.
///
/// # Panics
/// If Git cannot run or rejects the command.
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git").arg("-C").arg(root).args(args).output();
    let code = output
        .as_ref()
        .ok()
        .and_then(|result| return result.status.code());
    assert_eq!(
        code,
        Some(0_i32),
        "Git is required and must succeed: {output:?}"
    );
}

/// # Panics
/// If fixture setup fails, or a broken checkout silently falls back to a walk.
#[test]
fn broken_checkout_fails() {
    let fixture = fixture();
    drop(fs::create_dir_all(fixture.root.join(".git")));
    drop(fs::write(fixture.root.join("ignored.rs"), "ignored"));
    assert_eq!(
        files(&fixture.root).err().map(|error| return error.kind()),
        Some(io::ErrorKind::Other)
    );
}

/// # Panics
/// If enumeration fails to return a missing-root error to its caller.
#[test]
fn enumeration_errors_reach_the_caller() {
    let fixture = fixture();
    let reason = required(&fixture.root.join("missing"))
        .err()
        .map(|error| return error.kind());
    assert_eq!(reason, Some(io::ErrorKind::NotFound));
}

/// # Panics
/// If fixture setup or enumeration fails, or an ignored file is included.
#[test]
fn git_files_exclude_ignored_files() {
    let fixture = fixture();
    git(&fixture.root, &["init", "--quiet"]);
    drop(fs::write(fixture.root.join("tracked.rs"), "tracked"));
    drop(fs::write(fixture.root.join("ignored.rs"), "ignored"));
    drop(fs::write(
        fixture.root.join(".gitignore"),
        "ignored.rs\n.gitignore\n",
    ));
    git(&fixture.root, &["add", "tracked.rs"]);
    let found = files(&fixture.root).unwrap_or_default();
    assert_eq!(found, vec![fixture.root.join("tracked.rs")]);
}

/// # Panics
/// If fixture setup fails, or a non-Git file is excluded.
#[test]
fn outside_git_every_file_counts() {
    let fixture = fixture();
    drop(fs::write(fixture.root.join("tracked.rs"), "tracked"));
    drop(fs::write(fixture.root.join("ignored.rs"), "ignored"));
    drop(fs::write(
        fixture.root.join(".gitignore"),
        "ignored.rs\n.gitignore\n",
    ));
    let found = files(&fixture.root).unwrap_or_default();
    assert_eq!(found, vec![
        fixture.root.join(".gitignore"),
        fixture.root.join("ignored.rs"),
        fixture.root.join("tracked.rs"),
    ]);
}

/// # Panics
/// If fixture setup fails, or untracked files or tracked ignored files disappear.
#[test]
fn untracked_and_force_tracked_files_count() {
    let fixture = fixture();
    git(&fixture.root, &["init", "--quiet"]);
    drop(fs::write(
        fixture.root.join(".gitignore"),
        "tracked.rs\n.gitignore\n",
    ));
    drop(fs::write(fixture.root.join("tracked.rs"), "tracked"));
    drop(fs::write(fixture.root.join("untracked.rs"), "untracked"));
    git(&fixture.root, &["add", "--force", "tracked.rs"]);
    let found = files(&fixture.root).unwrap_or_default();
    assert_eq!(found, vec![
        fixture.root.join("tracked.rs"),
        fixture.root.join("untracked.rs"),
    ]);
}
