//! In-house source analyzers beyond the bundled linters.
//!
//! Adds duplicate-value const detection (dupconst parity), hash-gibberish
//! identifier detection (idiom parity), and unguarded float-division detection
//! (floatdiv parity), each over the crate's hand-written `src/` rust, reporting
//! advisory-then-gating findings the standard linters miss.

use alloc::collections::BTreeMap;
use std::{fs, path::PathBuf, process::Command};

/// Minimum distinct names sharing a value before it counts as a duplicate.
const MIN_DUP_NAMES: usize = 2;
/// Minimum gibberish-identifier length worth flagging.
const MIN_GIBBERISH_LEN: usize = 8;
/// Minimum literal length before a duplicate const value is worth flagging.
const MIN_VALUE_LEN: usize = 3;

/// A group of consts sharing one string value.
#[derive(Debug)]
pub struct DupConst {
    /// The const names bound to it.
    pub names: Vec<String>,
    /// The shared literal value.
    pub value: String,
}

/// A float division with no nearby zero/empty guard.
#[derive(Debug)]
pub struct FloatDiv {
    /// The denominator text.
    pub operand: String,
    /// `file:line` location.
    pub pos: String,
}

/// A hash-gibberish identifier and the file it was found in.
#[derive(Debug)]
pub struct Gibberish {
    /// File it appears in.
    pub file: String,
    /// The offending identifier.
    pub name: String,
}

/// Extracts the binding name after a `let `/`fn `/`const ` keyword on a line.
fn binding_name(line: &str, keyword: &str) -> Option<String> {
    let Some(after_kw) = line.trim_start().strip_prefix(keyword) else {
        return None;
    };
    let trimmed = after_kw.trim_start();
    let body = trimmed.strip_prefix("mut ").unwrap_or(trimmed);
    let name: String = body
        .chars()
        .take_while(|ch| return is_ident_char(*ch))
        .collect();
    if name.is_empty() {
        return None;
    }
    return Some(name);
}

/// Detects consts in the same file sharing one string value.
#[inline]
#[must_use]
pub fn dupconst() -> Vec<DupConst> {
    let mut by_value: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (_, text) in sources() {
        collect_consts(&text, &mut by_value);
    }
    let mut out = Vec::new();
    for (value, mut names) in by_value {
        names.sort();
        names.dedup();
        if names.len() >= MIN_DUP_NAMES {
            out.push(DupConst { names, value });
        }
    }
    return out;
}

/// Records every `const NAME = "value"` pair in one file's text.
fn collect_consts(text: &str, by_value: &mut BTreeMap<String, Vec<String>>) {
    for line in text.lines() {
        let Some((name, value)) = parse_const(line) else {
            continue;
        };
        by_value.entry(value).or_default().push(name);
    }
}

/// Detects unguarded float divisions across source.
#[inline]
#[must_use]
pub fn floatdiv() -> Vec<FloatDiv> {
    let mut out = Vec::new();
    for (file, text) in sources() {
        collect_float_divs(&file, &text, &mut out);
    }
    return out;
}

/// Records each risky float-division site in one file's text.
fn collect_float_divs(file: &str, text: &str, out: &mut Vec<FloatDiv>) {
    for (idx, line) in text.lines().enumerate() {
        let Some(operand) = risky_float_div(line) else {
            continue;
        };
        let pos = format!("{file}:{}", idx.saturating_add(1));
        out.push(FloatDiv { operand, pos });
    }
}

/// Renders the unguarded-float-division advisory block, empty when none.
#[inline]
#[must_use]
pub fn format_floatdiv(issues: &[FloatDiv]) -> String {
    if issues.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for issue in issues {
        out.push_str("  ");
        out.push_str(&issue.pos);
        out.push_str(": float division by ");
        out.push_str(&issue.operand);
        out.push_str(" with no zero/empty guard \u{2014} NaN/Inf risk on empty input\n");
    }
    return out;
}

/// Renders the duplicate-const advisory block, empty when none.
#[inline]
#[must_use]
pub fn format_dupconst(issues: &[DupConst]) -> String {
    if issues.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for issue in issues {
        out.push_str("  ");
        out.push_str(&issue.names.join(" = "));
        out.push_str(" = \"");
        out.push_str(&issue.value);
        out.push_str("\" \u{2014} duplicate-value consts; collapse to one\n");
    }
    return out;
}

/// Renders the gibberish-identifier advisory line, empty when none.
#[inline]
#[must_use]
pub fn format_gibberish(issues: &[Gibberish]) -> String {
    if issues.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = issues
        .iter()
        .map(|issue| return issue.name.as_str())
        .collect();
    let mut out = String::from("hash-gibberish identifiers (rename to self-documenting names): ");
    out.push_str(&names.join(", "));
    out.push('\n');
    return out;
}

/// Detects hash-gibberish binding/function/const identifiers across source.
#[inline]
#[must_use]
pub fn gibberish() -> Vec<Gibberish> {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (file, text) in sources() {
        collect_gibberish(&file, &text, &mut seen);
    }
    return seen
        .into_iter()
        .map(|(name, file)| return Gibberish { file, name })
        .collect();
}

/// Records first-seen gibberish identifiers in one file's text.
fn collect_gibberish(file: &str, text: &str, seen: &mut BTreeMap<String, String>) {
    for line in text.lines() {
        if let Some(name) = gibberish_on_line(line)
            && !seen.contains_key(&name)
        {
            drop(seen.insert(name, file.to_owned()));
        }
    }
}

/// The first gibberish binding name on a line, when any.
fn gibberish_on_line(line: &str) -> Option<String> {
    for keyword in ["let ", "fn ", "const "] {
        if let Some(name) = binding_name(line, keyword)
            && looks_gibberish(&name)
        {
            return Some(name);
        }
    }
    return None;
}

/// Whether a prefix ends inside a `"..."` string literal.
///
/// A following `/` is then string content, not a division operator, so the
/// analyzer never flags `as f64 / x.len()` written inside a test/format string.
fn in_string_literal(prefix: &str) -> bool {
    let mut open = false;
    let mut escaped = false;
    for ch in prefix.chars() {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            open = !open;
        } else {
            escaped = false;
        }
    }
    return open;
}

/// Whether a char can appear inside a rust identifier.
const fn is_ident_char(ch: char) -> bool {
    return ch == '_' || ch.is_ascii_alphanumeric();
}

/// Whether a line carries an inline zero/empty guard for its denominator.
fn is_guarded(line: &str) -> bool {
    return line.contains("> 0")
        || line.contains(">0")
        || line.contains("== 0")
        || line.contains("!= 0")
        || line.contains("is_empty")
        || line.contains("max(1")
        || line.contains("NonZero")
        || line.contains("checked_div");
}

/// Whether an identifier looks like a hash/gibberish token: a long no-underscore
/// run that mixes case and is digit-dense, the shape of a base-N hash.
fn looks_gibberish(name: &str) -> bool {
    if name.len() < MIN_GIBBERISH_LEN || name.contains('_') {
        return false;
    }
    if !name.chars().all(is_ident_char) {
        return false;
    }
    let has_digit = name.chars().any(|ch| return ch.is_ascii_digit());
    let has_upper = name.chars().any(|ch| return ch.is_ascii_uppercase());
    let has_lower = name.chars().any(|ch| return ch.is_ascii_lowercase());
    let digits = name.chars().filter(|ch| return ch.is_ascii_digit()).count();
    let dense = digits.saturating_mul(2) >= name.len();
    return has_digit && has_upper && has_lower && dense;
}

/// Pulls a `const NAME: T = "value";` name/value pair out of one line.
fn parse_const(line: &str) -> Option<(String, String)> {
    let Some(trimmed) = line.trim_start().strip_prefix("const ") else {
        return None;
    };
    let Some((raw_name, rest)) = trimmed.split_once(':') else {
        return None;
    };
    let name = raw_name.trim();
    if name.is_empty() || !name.chars().all(is_ident_char) {
        return None;
    }
    let Some(value) = const_value(rest) else {
        return None;
    };
    if value.len() < MIN_VALUE_LEN {
        return None;
    }
    return Some((name.to_owned(), value));
}

/// Extracts the first `"..."` literal value from the right side of a const.
fn const_value(rest: &str) -> Option<String> {
    let Some((_, after_eq)) = rest.split_once('=') else {
        return None;
    };
    let Some(open) = after_eq.find('"') else {
        return None;
    };
    let Some(body) = after_eq.get(open.saturating_add(1)..) else {
        return None;
    };
    let Some(close) = body.find('"') else {
        return None;
    };
    return body.get(..close).map(str::to_owned);
}

/// Returns the unguarded denominator text when a line divides a float by a
/// count-like value with no zero/empty guard present.
fn risky_float_div(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.starts_with("//") {
        return None;
    }
    let is_float = trimmed.contains("as f64") || trimmed.contains("as f32");
    if !is_float || !trimmed.contains('/') {
        return None;
    }
    let Some((before, after)) = trimmed.split_once('/') else {
        return None;
    };
    if in_string_literal(before) || is_guarded(trimmed) {
        return None;
    }
    let denom = after.trim().trim_start_matches('(').trim();
    if !denom.contains(".len()") && !denom.contains(".count()") {
        return None;
    }
    let head = denom
        .split(" as ")
        .next()
        .unwrap_or(denom)
        .split([',', ';'])
        .next()
        .unwrap_or(denom);
    return Some(head.trim().to_owned());
}

/// Hand-written rust source files under `src/` (the gate's scan scope).
#[inline]
#[must_use]
pub fn source_files() -> Vec<PathBuf> {
    let Ok(output) = Command::new("rg")
        .args(["--files", "-t", "rust", "src/"])
        .output()
    else {
        return Vec::new();
    };
    return String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|line| return !line.is_empty())
        .map(PathBuf::from)
        .collect();
}

/// Reads each source file's `(path, text)`, skipping unreadable entries.
fn sources() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for path in source_files() {
        if let Ok(text) = fs::read_to_string(&path) {
            out.push((path.display().to_string(), text));
        }
    }
    return out;
}

#[cfg(test)]
mod tests {
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
}
