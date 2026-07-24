use super::normalize_dprint;

/// # Panics
/// On assertion failure.
#[test]
fn normalize_strips_plugin_version() {
    let pinned = "\"https://plugins.dprint.dev/toml-0.7.0.wasm\",";
    let other = "\"https://plugins.dprint.dev/toml-0.9.9.wasm\",";
    assert_eq!(normalize_dprint(pinned), normalize_dprint(other));
}
