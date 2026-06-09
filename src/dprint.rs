//! Never-stale dprint plugin resolution: rewrites pinned plugin URLs in the
//! written config to each plugin's current latest, so the embedded version
//! pins are only a bootstrap seed.

use std::process::Command;

/// Marker prefix every dprint plugin URL shares.
const HOST: &str = "https://plugins.dprint.dev/";

/// Plugin name from a wasm filename (`toml-0.7.0`, `malva-v0.16.0`) by dropping
/// the version suffix.
fn plugin_name(file: &str) -> &str {
    return file.split(['-', '@']).next().unwrap_or(file);
}

/// Extracts the plugin path (e.g. `dprint/toml`, `g-plane/malva`) from a pinned
/// wasm URL so its `latest.json` can be fetched.
fn plugin_path(url: &str) -> Option<String> {
    let Some(tail) = url.strip_prefix(HOST) else {
        return None;
    };
    let Some(file) = tail.strip_suffix(".wasm") else {
        return None;
    };
    if let Some((owner, rest)) = file.split_once('/') {
        return Some(format!("{owner}/{}", plugin_name(rest)));
    }
    return Some(format!("dprint/{}", plugin_name(file)));
}

/// Fetches the latest wasm URL for a plugin path via its `latest.json`.
fn latest_url(path: &str) -> Option<String> {
    let endpoint = format!("{HOST}{path}/latest.json");
    let Ok(output) = Command::new("curl").args(["-fsSL", &endpoint]).output() else {
        return None;
    };
    if !output.status.success() {
        return None;
    }
    let Ok(body) = String::from_utf8(output.stdout) else {
        return None;
    };
    return extract_url(&body);
}

/// Pulls the `"url"` field out of a `latest.json` body without a JSON dependency
/// beyond a single-field scan (the body is a flat object).
fn extract_url(body: &str) -> Option<String> {
    let Some((_, after)) = body.split_once("\"url\":") else {
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
    let Some(url) = rest.get(..close) else {
        return None;
    };
    if url.starts_with(HOST) {
        return Some(url.to_owned());
    }
    return None;
}

/// Rewrites a single config line to the plugin's latest URL when it pins one.
fn rewrite_line(line: &str) -> String {
    let trimmed = line.trim();
    let inner = trimmed.trim_matches(|ch| return ch == '"' || ch == ',');
    if !inner.starts_with(HOST) {
        return line.to_owned();
    }
    let Some(path) = plugin_path(inner) else {
        return line.to_owned();
    };
    let Some(latest) = latest_url(&path) else {
        return line.to_owned();
    };
    return line.replace(inner, &latest);
}

/// Returns the config text with every plugin URL bumped to latest, or `None`
/// when nothing changed (so callers can skip a needless write).
#[inline]
#[must_use]
pub fn bump(config: &str) -> Option<String> {
    let bumped = config
        .lines()
        .map(rewrite_line)
        .collect::<Vec<_>>()
        .join("\n");
    let restored = if config.ends_with('\n') {
        format!("{bumped}\n")
    } else {
        bumped
    };
    if restored == config {
        return None;
    }
    return Some(restored);
}

#[cfg(test)]
mod tests {
    use super::extract_url;
    use super::plugin_path;

    /// # Panics
    /// On assertion failure.
    #[test]
    fn extracts_url_field() {
        let body = r#"{"schemaVersion":1,"url":"https://plugins.dprint.dev/toml-0.7.0.wasm","version":"0.7.0"}"#;
        assert_eq!(
            extract_url(body),
            Some("https://plugins.dprint.dev/toml-0.7.0.wasm".to_owned())
        );
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn plugin_path_for_gplane() {
        let url = "https://plugins.dprint.dev/g-plane/malva-v0.15.2.wasm";
        assert_eq!(plugin_path(url), Some("g-plane/malva".to_owned()));
    }

    /// # Panics
    /// On assertion failure.
    #[test]
    fn plugin_path_for_official() {
        let url = "https://plugins.dprint.dev/toml-0.7.0.wasm";
        assert_eq!(plugin_path(url), Some("dprint/toml".to_owned()));
    }
}
