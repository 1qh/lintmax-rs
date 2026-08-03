use super::{extract_url, plugin_path};

/// # Panics
/// On assertion failure.
#[test]
fn extracts_url_field() {
    let body = concat!(
        r#"{"schemaVersion":1,"#,
        r#""url":"https://plugins.dprint.dev/toml-0.7.0.wasm","#,
        r#""version":"0.7.0"}"#,
    );
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

/// A hyphenated plugin name survives resolution.
///
/// Splitting at the first separator would resolve `lax-sql` to `lax`.
///
/// # Panics
/// On assertion failure.
#[test]
fn plugin_path_keeps_a_hyphenated_name() {
    let url = "https://plugins.dprint.dev/bartlomieju/lax-sql-0.3.0.wasm";
    assert_eq!(plugin_path(url), Some("bartlomieju/lax-sql".to_owned()));
}

/// # Panics
/// On assertion failure.
#[test]
fn plugin_path_keeps_an_underscored_name() {
    let url = "https://plugins.dprint.dev/g-plane/markup_fmt-v0.27.3.wasm";
    assert_eq!(plugin_path(url), Some("g-plane/markup_fmt".to_owned()));
}
