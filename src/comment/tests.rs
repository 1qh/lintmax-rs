use super::strip_line;

/// # Panics
/// On assertion failure.
#[test]
fn handles_escaped_quote_in_string() {
    let line = r#"let s = "a\"// b"; // tail"#;
    let (out, removed) = strip_line(line);
    assert_eq!(out, r#"let s = "a\"// b";"#);
    assert!(removed);
}

/// # Panics
/// On assertion failure.
#[test]
fn ignores_slashes_in_char_literal() {
    let line = "let c = '/';";
    assert!(!strip_line(line).1);
}

/// # Panics
/// On assertion failure.
#[test]
fn ignores_slashes_in_raw_string() {
    let line = r##"let u = r#"a // b"#;"##;
    assert!(!strip_line(line).1);
}

/// # Panics
/// On assertion failure.
#[test]
fn ignores_slashes_in_string() {
    let line = r#"let u = "http://example.com/x"; // real comment"#;
    let (out, removed) = strip_line(line);
    assert_eq!(out, r#"let u = "http://example.com/x";"#);
    assert!(removed);
}

/// # Panics
/// On assertion failure.
#[test]
fn keeps_doc_comments() {
    assert!(!strip_line("/// doc").1);
    assert!(!strip_line("//! inner doc").1);
    assert!(!strip_line("    /// indented doc").1);
}

/// # Panics
/// On assertion failure.
#[test]
fn keeps_plain_code() {
    let (out, removed) = strip_line("let x = 1;");
    assert_eq!(out, "let x = 1;");
    assert!(!removed);
}

/// # Panics
/// On assertion failure.
#[test]
fn keeps_url_only_string() {
    let line = r#"let u = "https://a.b/c//d";"#;
    let (out, removed) = strip_line(line);
    assert_eq!(out, line);
    assert!(!removed);
}

/// # Panics
/// On assertion failure.
#[test]
fn lifetime_does_not_hide_comment() {
    let line = "fn f<'a>(x: &'a str) {} // tail";
    let (out, removed) = strip_line(line);
    assert_eq!(out, "fn f<'a>(x: &'a str) {}");
    assert!(removed);
}

/// # Panics
/// On assertion failure.
#[test]
fn strips_line_leading_comment_to_empty() {
    let (out, removed) = strip_line("    // a note");
    assert_eq!(out, "");
    assert!(removed);
}

/// # Panics
/// On assertion failure.
#[test]
fn strips_trailing_inline_comment() {
    let (out, removed) = strip_line("let x = 1; // set x");
    assert_eq!(out, "let x = 1;");
    assert!(removed);
}
