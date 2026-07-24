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
