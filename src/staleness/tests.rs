use super::{json_string_field, major, parse_dep_line, same_major};

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
