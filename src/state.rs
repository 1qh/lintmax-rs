//! Green tree-hash cache.
//!
//! Records the working-tree hash of the last clean gate run per directory so an
//! unchanged re-run short-circuits with `ok (cached)`.

use alloc::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use serde::Serialize;

/// Cache subdirectory under the OS cache root.
const APP_DIR: &str = "lintmax-rs";
/// Toolchain-refresh window in seconds (one day).
const REFRESH_WINDOW: u64 = 86_400;
/// FNV-1a 64-bit offset basis.
const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
/// State file name within the cache directory.
const FILE_NAME: &str = "state.json";

/// Persisted lintmax state: last green tree hash per working directory and the
/// last staleness-check timestamp (unix seconds).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// Unix seconds of the last staleness registry check.
    #[serde(default)]
    pub last_check: u64,
    /// Working directory → tree hash of its last clean gate run.
    #[serde(default)]
    pub last_green_by_cwd: BTreeMap<String, String>,
}

impl State {
    /// Persists state to the cache file, best-effort (a write failure never
    /// fails the gate — the cache is an optimization, not a contract).
    #[inline]
    pub fn save(&self) {
        let Some(path) = state_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            drop(fs::create_dir_all(parent));
        }
        if let Ok(raw) = serde_json::to_string_pretty(self) {
            drop(fs::write(path, raw));
        }
    }
}

/// Unix seconds now, zero when the clock is unreadable.
fn now_secs() -> u64 {
    return SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |dur| return dur.as_secs());
}

/// Whether the toolchain-refresh window has elapsed since the last refresh.
#[inline]
#[must_use]
pub fn refresh_due() -> bool {
    return now_secs().saturating_sub(load().last_check) >= REFRESH_WINDOW;
}

/// Records the current time as the last toolchain refresh.
#[inline]
pub fn mark_refreshed() {
    let mut st = load();
    st.last_check = now_secs();
    st.save();
}

/// Current working directory as a string key, when resolvable.
#[inline]
#[must_use]
pub fn cwd_key() -> Option<String> {
    return env::current_dir()
        .ok()
        .map(|path| return path.display().to_string());
}

/// OS user cache directory, honoring `XDG_CACHE_HOME` then a macOS
/// `Library/Caches` / `HOME/.cache` fallback so a state home always resolves.
fn dirs_cache() -> Option<PathBuf> {
    if let Ok(xdg) = env::var("XDG_CACHE_HOME")
        && !xdg.is_empty()
    {
        return Some(PathBuf::from(xdg));
    }
    let Some(home) = env::var("HOME").ok().filter(|val| return !val.is_empty()) else {
        return None;
    };
    let mut base = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        base.push("Library");
        base.push("Caches");
    } else {
        base.push(".cache");
    }
    return Some(base);
}

/// FNV-1a 64-bit digest — a dependency-free content hash sufficient for
/// change-detection (the cache only needs to differ on any edit).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    return hash;
}

/// Formats a 64-bit value as a fixed 16-char lowercase hex string.
fn hex16(value: u64) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    const NIBBLES: usize = 16;
    let mut out = String::with_capacity(NIBBLES);
    for shift in (0..NIBBLES).rev() {
        let nibble = (value >> (shift.saturating_mul(4))) & 0xf;
        let Ok(idx) = usize::try_from(nibble) else {
            continue;
        };
        if let Some(&byte) = HEX.get(idx) {
            out.push(char::from(byte));
        }
    }
    return out;
}

/// Loads persisted state, returning the default when absent or unreadable so a
/// missing cache never fails the gate.
#[inline]
#[must_use]
pub fn load() -> State {
    let Some(path) = state_path() else {
        return State::default();
    };
    let Ok(raw) = fs::read_to_string(path) else {
        return State::default();
    };
    return serde_json::from_str(&raw).unwrap_or_default();
}

/// Path to the persisted state file, when a cache directory is resolvable.
fn state_path() -> Option<PathBuf> {
    let Some(mut dir) = dirs_cache() else {
        return None;
    };
    dir.push(APP_DIR);
    dir.push(FILE_NAME);
    return Some(dir);
}

/// Hashes the tracked working tree: the lintmax version tag plus every
/// `git ls-files` path and its content, so any source edit changes the digest.
#[inline]
#[must_use]
pub fn tree_hash(version: &str) -> Option<String> {
    let Ok(output) = Command::new("git").args(["ls-files", "-z"]).output() else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let Ok(listing) = String::from_utf8(output.stdout) else {
        return None;
    };
    let mut acc = String::from("lintmax-rs:");
    acc.push_str(version);
    acc.push('\n');
    for file in listing.split('\0').filter(|path| return !path.is_empty()) {
        let digest = fs::read(file).map_or(0, |body| return fnv1a(&body));
        acc.push_str(file);
        acc.push(':');
        acc.push_str(&hex16(digest));
        acc.push('\n');
    }
    return Some(hex16(fnv1a(acc.as_bytes())));
}

#[cfg(test)]
mod tests {
    use super::fnv1a;

    /// # Panics
    /// On assertion failure.
    #[test]
    fn fnv1a_differs_on_change() {
        assert_ne!(fnv1a(b"alpha"), fnv1a(b"alpha2"));
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn fnv1a_stable() {
        assert_eq!(fnv1a(b"lintmax"), fnv1a(b"lintmax"));
    }
}
