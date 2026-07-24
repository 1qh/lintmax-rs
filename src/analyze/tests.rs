use super::{is_guarded, looks_gibberish, parse_const, risky_float_div};

/// # Panics
/// On assertion failure.
#[test]
fn flags_unguarded_float_div() {
    let line = "let mean = total as f64 / items.len() as f64;";
    assert_eq!(risky_float_div(line), Some("items.len()".to_owned()));
}

/// # Panics
/// On assertion failure.
#[test]
fn gibberish_detects_hashy_name() {
    assert!(looks_gibberish("a1B2c3D4e5"));
    assert!(!looks_gibberish("tree_hash"));
    assert!(!looks_gibberish("config"));
}

/// # Panics
/// On assertion failure.
#[test]
fn guarded_skips_protected_div() {
    let line = "let mean = if n > 0 { total as f64 / n.len() as f64 } else { 0.0 };";
    assert!(is_guarded(line));
    assert_eq!(risky_float_div(line), None);
}

/// # Panics
/// On assertion failure.
#[test]
fn parses_string_const() {
    let parsed = parse_const(r#"    const APP_DIR: &str = "lintmax-rs";"#);
    assert_eq!(
        parsed,
        Some(("APP_DIR".to_owned(), "lintmax-rs".to_owned()))
    );
}
