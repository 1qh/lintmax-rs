//! Realtime active-maintenance scan.
//!
//! Checks the project's direct crate deps against crates.io and the GitHub
//! Actions pins against GitHub releases, warning (advisory, never failing) when
//! a pin is behind the active-maintenance window.

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;

/// Environment flag that disables the scan entirely.
const ENV_SKIP: &str = "LINTMAX_SKIP_STALENESS";
/// User-Agent crates.io and the GitHub API require for unauthenticated reads.
const USER_AGENT: &str = "lintmax-rs (https://github.com/1qh/lintmax-rs)";

/// A `(name, version)` dependency pin.
type Pin = (String, String);

/// A function that turns one pin into a behind-latest issue, when it lags.
type Resolver = fn(Pin) -> Option<Issue>;

/// One out-of-date dependency pin.
#[derive(Debug)]
pub struct Issue {
    /// Pinned version.
    pub have: String,
    /// Latest available version.
    pub latest: String,
    /// Dependency name.
    pub name: String,
    /// Where the pin lives (`crate` or `action`).
    pub source: String,
}

/// Produces an issue when an action's pinned major lags its latest release tag.
fn action_issue(action: &str, have: &str) -> Option<Issue> {
    let url = format!("https://api.github.com/repos/{action}/releases/latest");
    let Some(body) = fetch(&url) else {
        return None;
    };
    let Some(latest) = json_string_field(&body, "tag_name") else {
        return None;
    };
    if same_major(have, &latest) {
        return None;
    }
    return Some(Issue {
        have: have.to_owned(),
        latest,
        name: action.to_owned(),
        source: "action".to_owned(),
    });
}

/// Reads the `[dependencies]` table from `Cargo.toml` as name/version pairs.
fn cargo_deps(root: &Path) -> Vec<(String, String)> {
    let Ok(text) = fs::read_to_string(root.join("Cargo.toml")) else {
        return Vec::new();
    };
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_deps = trimmed == "[dependencies]";
            continue;
        }
        if in_deps && let Some(pair) = parse_dep_line(trimmed) {
            deps.push(pair);
        }
    }
    return deps;
}

/// Extracts `owner/repo@vN` action pins from a workflow body.
fn collect_action_pins(text: &str, out: &mut Vec<(String, String)>) {
    for line in text.lines() {
        if let Some(pin) = action_pin(line) {
            out.push(pin);
        }
    }
}

/// Parses one workflow line into an `(owner/repo, version)` pin, when present.
fn action_pin(line: &str) -> Option<(String, String)> {
    let Some((_, after)) = line.split_once("uses:") else {
        return None;
    };
    let Some((raw_action, raw_version)) = after.trim().split_once('@') else {
        return None;
    };
    let action = raw_action.trim();
    if action.starts_with("./") || !action.contains('/') {
        return None;
    }
    let version = raw_version.split_whitespace().next().unwrap_or(raw_version);
    if !is_versionish(version) {
        return None;
    }
    let repo = action.split('/').take(2).collect::<Vec<_>>().join("/");
    return Some((repo, version.to_owned()));
}

/// Produces an issue when a crate's pinned major lags the crates.io latest.
fn crate_issue(name: &str, have: &str) -> Option<Issue> {
    let url = format!("https://crates.io/api/v1/crates/{name}");
    let Some(body) = fetch(&url) else {
        return None;
    };
    let Some(latest) = json_string_field(&body, "max_stable_version") else {
        return None;
    };
    if same_major(have, &latest) {
        return None;
    }
    return Some(Issue {
        have: have.to_owned(),
        latest,
        name: name.to_owned(),
        source: "crate".to_owned(),
    });
}

/// Fetches the body of a URL via curl with the required User-Agent.
fn fetch(url: &str) -> Option<String> {
    let Ok(output) = Command::new("curl")
        .args(["-fsSL", "-A", USER_AGENT, url])
        .output()
    else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    return String::from_utf8(output.stdout).ok();
}

/// Renders the behind-latest set as an advisory block, empty when nothing lags.
#[inline]
#[must_use]
pub fn format(issues: &[Issue]) -> String {
    if issues.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for issue in issues {
        out.push_str("  ");
        out.push_str(&pad(&issue.source, 8));
        out.push(' ');
        out.push_str(&pad(&issue.name, 32));
        out.push(' ');
        out.push_str(&issue.have);
        out.push_str(" \u{2192} ");
        out.push_str(&issue.latest);
        out.push('\n');
    }
    return out;
}

/// Whether a pin is a numeric version (`v6`, `1.2.3`) rather than a branch or
/// channel name (`stable`, `latest`, `main`) the registry cannot compare.
fn is_versionish(version: &str) -> bool {
    return version
        .trim_start_matches('v')
        .chars()
        .next()
        .is_some_and(|ch| return ch.is_ascii_digit());
}

/// Whether a path is a YAML workflow file.
fn is_yaml(path: &Path) -> bool {
    return path
        .extension()
        .and_then(|ext| return ext.to_str())
        .is_some_and(|ext| return ext == "yml" || ext == "yaml");
}

/// Pulls a string field's value out of a flat JSON body without a typed model.
fn json_string_field(body: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\":");
    let Some((_, after)) = body.split_once(&needle) else {
        return None;
    };
    let Some(open) = after.find('"') else {
        return None;
    };
    let Some(rest) = after.get(open.saturating_add(1)..) else {
        return None;
    };
    let Some(close) = rest.find('"') else {
        return None;
    };
    return rest.get(..close).map(str::to_owned);
}

/// Leading numeric major of a version/tag (`v6`, `1.2.3` → `6`, `1`).
fn major(version: &str) -> String {
    let trimmed = version.trim_start_matches('v');
    return trimmed
        .split(['.', '+', '-', ' '])
        .next()
        .unwrap_or(trimmed)
        .to_owned();
}

/// Right-pads a string to a minimum width with spaces.
fn pad(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    while out.chars().count() < width {
        out.push(' ');
    }
    return out;
}

/// Parses one `name = "1.2.3"` or `name = { version = "1.2.3", … }` line.
fn parse_dep_line(line: &str) -> Option<(String, String)> {
    let Some((raw_name, rhs)) = line.split_once('=') else {
        return None;
    };
    let name = raw_name.trim();
    if name.is_empty() || name.starts_with('#') {
        return None;
    }
    let body = rhs.trim();
    let version = if body.starts_with('{') {
        let Some(found) = toml_string_field(body, "version") else {
            return None;
        };
        found
    } else {
        body.trim_matches(|ch| return ch == '"' || ch == ',')
            .to_owned()
    };
    if version.is_empty() {
        return None;
    }
    return Some((name.to_owned(), version));
}

/// Pulls a `key = "value"` field out of a TOML inline table body.
fn toml_string_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("{key} =");
    let Some((_, after)) = body.split_once(&needle) else {
        return None;
    };
    let Some(open) = after.find('"') else {
        return None;
    };
    let Some(rest) = after.get(open.saturating_add(1)..) else {
        return None;
    };
    let Some(close) = rest.find('"') else {
        return None;
    };
    return rest.get(..close).map(str::to_owned);
}

/// Whether two version strings share a leading major component.
fn same_major(left: &str, right: &str) -> bool {
    return major(left) == major(right);
}

/// Scans every dep source in parallel, returning the merged behind-latest set.
///
/// Returns an empty set when skipped or when no registry is reachable, so the
/// scan is always advisory and never blocks a clean gate.
#[inline]
#[must_use]
pub fn scan(root: &Path) -> Vec<Issue> {
    if env::var(ENV_SKIP).is_ok_and(|val| return val == "1") {
        return Vec::new();
    }
    let crates_root = root.to_path_buf();
    let actions_root = root.to_path_buf();
    let crates_handle = thread::spawn(move || return scan_crates(&crates_root));
    let actions = scan_actions(&actions_root);
    let mut merged = crates_handle.join().unwrap_or_default();
    merged.extend(actions);
    return merged;
}

/// Checks GitHub Actions `uses:` pins against each action's latest release.
fn scan_actions(root: &Path) -> Vec<Issue> {
    let dir = root.join(".github").join("workflows");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut pins: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if is_yaml(&path)
            && let Ok(text) = fs::read_to_string(&path)
        {
            collect_action_pins(&text, &mut pins);
        }
    }
    pins.sort();
    pins.dedup();
    return resolve(pins, |pair| return action_issue(&pair.0, &pair.1));
}

/// Checks each direct crate dep against its crates.io latest stable version.
fn scan_crates(root: &Path) -> Vec<Issue> {
    return resolve(cargo_deps(root), |pair| {
        return crate_issue(&pair.0, &pair.1);
    });
}

/// Resolves each `(name, version)` pin concurrently through `check`, keeping the
/// issues it produces.
fn resolve(pins: Vec<Pin>, check: Resolver) -> Vec<Issue> {
    let handles: Vec<_> = pins
        .into_iter()
        .map(|pin| return thread::spawn(move || return check(pin)))
        .collect();
    let mut out = Vec::new();
    for handle in handles {
        if let Ok(Some(issue)) = handle.join() {
            out.push(issue);
        }
    }
    return out;
}

#[cfg(test)]
mod tests {
    use super::json_string_field;
    use super::major;
    use super::parse_dep_line;
    use super::same_major;

    /// # Panics
    /// On assertion failure.
    #[test]
    fn extracts_max_stable_version() {
        let body = r#"{"crate":{"max_stable_version":"1.0.228"}}"#;
        assert_eq!(
            json_string_field(body, "max_stable_version"),
            Some("1.0.228".to_owned())
        );
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn major_strips_v_prefix() {
        assert_eq!(major("v6"), "6");
        assert_eq!(major("1.2.3"), "1");
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn parses_inline_table_dep() {
        let parsed = parse_dep_line(r#"clap = { version = "4.6.0", features = ["derive"] }"#);
        assert_eq!(parsed, Some(("clap".to_owned(), "4.6.0".to_owned())));
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn parses_plain_dep() {
        let parsed = parse_dep_line(r#"serde_json = "1.0.150""#);
        assert_eq!(
            parsed,
            Some(("serde_json".to_owned(), "1.0.150".to_owned()))
        );
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn same_major_matches_across_minor() {
        assert!(same_major("1.0.1", "1.9.9"));
        assert!(!same_major("1.0.0", "2.0.0"));
    }
}
