//! Realtime active-maintenance scan.
//!
//! Checks the project's direct crate deps against crates.io and the GitHub
//! Actions pins against GitHub releases, warning (advisory, never failing) when
//! a pin is behind the active-maintenance window.

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    thread,
};

use crate::files::required;

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
#[non_exhaustive]
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
    let body = fetch(&url)?;
    let latest = json_string_field(&body, "tag_name")?;
    if same_major(have, &latest) {
        return None;
    }
    Some(Issue {
        have: have.to_owned(),
        latest,
        name: action.to_owned(),
        source: "action".to_owned(),
    })
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
    deps
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
    let (_, after) = line.split_once("uses:")?;
    let (raw_action, raw_version) = after.trim().split_once('@')?;
    let action = raw_action.trim();
    if action.starts_with("./") || !action.contains('/') {
        return None;
    }
    let version = raw_version.split_whitespace().next().unwrap_or(raw_version);
    if !is_versionish(version) {
        return None;
    }
    let repo = action.split('/').take(2).collect::<Vec<_>>().join("/");
    Some((repo, version.to_owned()))
}

/// Produces an issue when a crate's pinned major lags the crates.io latest.
fn crate_issue(name: &str, have: &str) -> Option<Issue> {
    let url = format!("https://crates.io/api/v1/crates/{name}");
    let body = fetch(&url)?;
    let latest = json_string_field(&body, "max_stable_version")?;
    if same_major(have, &latest) {
        return None;
    }
    Some(Issue {
        have: have.to_owned(),
        latest,
        name: name.to_owned(),
        source: "crate".to_owned(),
    })
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
    String::from_utf8(output.stdout).ok()
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
    out
}

/// Whether a pin is a numeric version (`v6`, `1.2.3`) rather than a branch or
/// channel name (`stable`, `latest`, `main`) the registry cannot compare.
fn is_versionish(version: &str) -> bool {
    version
        .trim_start_matches('v')
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_digit())
}

/// Whether a workflow is one a GENERATOR owns rather than this project.
///
/// A pin inside generated output belongs to the tool that emits it, so bumping
/// it here is hand-editing that tool's output and the next generate undoes it —
/// the honest owner is the generator's own version. Compared on the path's
/// SUFFIX so a declaration written relative to the repo root matches the
/// absolute path the scan walks.
fn is_generated(path: &Path, generated: &[String]) -> bool {
    let text = path.to_string_lossy().replace('\\', "/");
    generated
        .iter()
        .any(|owned| text.ends_with(owned.trim_start_matches("./")))
}

/// Whether a path is a YAML workflow file.
fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "yml" || ext == "yaml")
}

/// Pulls a string field's value out of a flat JSON body without a typed model.
fn json_string_field(body: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\":");
    let (_, after) = body.split_once(&needle)?;
    let open = after.find('"')?;
    let rest = after.get(open.saturating_add(1)..)?;
    let close = rest.find('"')?;
    rest.get(..close).map(str::to_owned)
}

/// Leading numeric major of a version/tag (`v6`, `1.2.3` → `6`, `1`).
fn major(version: &str) -> String {
    let trimmed = version.trim_start_matches('v');
    trimmed
        .split(['.', '+', '-', ' '])
        .next()
        .unwrap_or(trimmed)
        .to_owned()
}

/// Right-pads a string to a minimum width with spaces.
fn pad(text: &str, width: usize) -> String {
    let mut out = text.to_owned();
    while out.chars().count() < width {
        out.push(' ');
    }
    out
}

/// Parses one `name = "1.2.3"` or `name = { version = "1.2.3", … }` line.
fn parse_dep_line(line: &str) -> Option<(String, String)> {
    let (raw_name, rhs) = line.split_once('=')?;
    let name = raw_name.trim();
    if name.is_empty() || name.starts_with('#') {
        return None;
    }
    let body = rhs.trim();
    let version = if body.starts_with('{') {
        toml_string_field(body, "version")?
    } else {
        body.trim_matches(|ch| ch == '"' || ch == ',').to_owned()
    };
    if version.is_empty() {
        return None;
    }
    Some((name.to_owned(), version))
}

/// Pulls a `key = "value"` field out of a TOML inline table body.
fn toml_string_field(body: &str, key: &str) -> Option<String> {
    let needle = format!("{key} =");
    let (_, after) = body.split_once(&needle)?;
    let open = after.find('"')?;
    let rest = after.get(open.saturating_add(1)..)?;
    let close = rest.find('"')?;
    rest.get(..close).map(str::to_owned)
}

/// Whether two version strings share a leading major component.
fn same_major(left: &str, right: &str) -> bool {
    major(left) == major(right)
}

/// Scans every dep source in parallel, returning the merged behind-latest set.
///
/// Returns an empty set when skipped or when no registry is reachable. Outdated
/// pins remain advisory; file enumeration failures fail the stage.
///
/// # Errors
/// Returns an error if eligible workflow enumeration fails.
#[inline]
pub fn scan(root: &Path, generated: &[String]) -> io::Result<Vec<Issue>> {
    if env::var(ENV_SKIP).is_ok_and(|val| val == "1") {
        return Ok(Vec::new());
    }
    let paths = required(root)?;
    let crates_root = root.to_path_buf();
    let crates_handle = thread::spawn(move || scan_crates(&crates_root));
    let actions = scan_actions(root, generated, &paths);
    let mut merged = crates_handle.join().unwrap_or_default();
    merged.extend(actions);
    Ok(merged)
}

/// Checks eligible GitHub Actions pins against each action's latest release.
fn scan_actions(root: &Path, generated: &[String], paths: &[PathBuf]) -> Vec<Issue> {
    let dir = root.join(".github").join("workflows");
    let directory = fs::canonicalize(&dir).unwrap_or(dir);
    let mut pins: Vec<(String, String)> = Vec::new();
    for path in paths {
        collect_workflow(path, &directory, generated, &mut pins);
    }
    pins.sort();
    pins.dedup();
    resolve(pins, |pair| action_issue(&pair.0, &pair.1))
}

/// Collects pins from an eligible, non-generated workflow in the workflow directory.
fn collect_workflow(path: &Path, directory: &Path, generated: &[String], pins: &mut Vec<Pin>) {
    if path.parent() != Some(directory) || !is_yaml(path) || is_generated(path, generated) {
        return;
    }
    if let Ok(text) = fs::read_to_string(path) {
        collect_action_pins(&text, pins);
    }
}

/// Checks each direct crate dep against its crates.io latest stable version.
fn scan_crates(root: &Path) -> Vec<Issue> {
    resolve(cargo_deps(root), |pair| crate_issue(&pair.0, &pair.1))
}

/// Resolves each `(name, version)` pin concurrently through `check`, keeping the
/// issues it produces.
fn resolve(pins: Vec<Pin>, check: Resolver) -> Vec<Issue> {
    let handles: Vec<_> = pins
        .into_iter()
        .map(|pin| thread::spawn(move || check(pin)))
        .collect();
    let mut out = Vec::new();
    for handle in handles {
        if let Ok(Some(issue)) = handle.join() {
            out.push(issue);
        }
    }
    out
}

#[cfg(test)]
mod tests;
