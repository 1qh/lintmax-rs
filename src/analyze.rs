//! In-house source analyzers beyond the bundled linters.
//!
//! Adds duplicate-value const detection (dupconst parity), hash-gibberish
//! identifier detection (idiom parity), and unguarded float-division detection
//! (floatdiv parity), each over the crate's hand-written `src/` rust, reporting
//! advisory-then-gating findings the standard linters miss.

use alloc::collections::BTreeMap;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::files::required;

/// Minimum distinct names sharing a value before it counts as a duplicate.
const MIN_DUP_NAMES: usize = 2;
/// Minimum gibberish-identifier length worth flagging.
const MIN_GIBBERISH_LEN: usize = 8;
/// Minimum literal length before a duplicate const value is worth flagging.
const MIN_VALUE_LEN: usize = 3;

/// A group of consts sharing one string value.
#[derive(Debug)]
#[non_exhaustive]
pub struct DupConst {
    /// The const names bound to it.
    pub names: Vec<String>,
    /// The shared literal value.
    pub value: String,
}

/// A float division with no nearby zero/empty guard.
#[derive(Debug)]
#[non_exhaustive]
pub struct FloatDiv {
    /// The denominator text.
    pub operand: String,
    /// `file:line` location.
    pub pos: String,
}

/// A hash-gibberish identifier and the file it was found in.
#[derive(Debug)]
#[non_exhaustive]
pub struct Gibberish {
    /// File it appears in.
    pub file: String,
    /// The offending identifier.
    pub name: String,
}

/// Whether a char can appear inside a rust identifier.
const fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphanumeric()
}

/// Extracts the binding name after a `let `/`fn `/`const ` keyword on a line.
fn binding_name(line: &str, keyword: &str) -> Option<String> {
    let after_kw = line.trim_start().strip_prefix(keyword)?;
    let trimmed = after_kw.trim_start();
    let body = trimmed.strip_prefix("mut ").unwrap_or(trimmed);
    let name: String = body.chars().take_while(|ch| is_ident_char(*ch)).collect();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

/// Detects consts in the same file sharing one string value.
///
/// # Errors
/// Returns an error if source file enumeration fails.
#[inline]
pub fn dupconst() -> io::Result<Vec<DupConst>> {
    let mut by_value: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let texts = sources()?;
    for (_, text) in texts {
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
    Ok(out)
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
///
/// # Errors
/// Returns an error if source file enumeration fails.
#[inline]
pub fn floatdiv() -> io::Result<Vec<FloatDiv>> {
    let mut out = Vec::new();
    let texts = sources()?;
    for (file, text) in texts {
        collect_float_divs(&file, &text, &mut out);
    }
    Ok(out)
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
    out
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
    out
}

/// Renders the gibberish-identifier advisory line, empty when none.
#[inline]
#[must_use]
pub fn format_gibberish(issues: &[Gibberish]) -> String {
    if issues.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for issue in issues {
        out.push_str("  ");
        out.push_str(&issue.file);
        out.push_str(": ");
        out.push_str(&issue.name);
        out.push_str(" \u{2014} hash-gibberish identifier; rename to a self-documenting name\n");
    }
    out
}

/// Detects hash-gibberish binding/function/const identifiers across source.
///
/// # Errors
/// Returns an error if source file enumeration fails.
#[inline]
pub fn gibberish() -> io::Result<Vec<Gibberish>> {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    let texts = sources()?;
    for (file, text) in texts {
        collect_gibberish(&file, &text, &mut seen);
    }
    Ok(seen
        .into_iter()
        .map(|(name, file)| Gibberish { file, name })
        .collect())
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
    None
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
    open
}

/// Whether a line carries an inline zero/empty guard for its denominator.
fn is_guarded(line: &str) -> bool {
    line.contains("> 0")
        || line.contains(">0")
        || line.contains("== 0")
        || line.contains("!= 0")
        || line.contains("is_empty")
        || line.contains("max(1")
        || line.contains("NonZero")
        || line.contains("checked_div")
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
    let has_digit = name.chars().any(|ch| ch.is_ascii_digit());
    let has_upper = name.chars().any(|ch| ch.is_ascii_uppercase());
    let has_lower = name.chars().any(|ch| ch.is_ascii_lowercase());
    let digits = name.chars().filter(char::is_ascii_digit).count();
    let dense = digits.saturating_mul(2) >= name.len();
    has_digit && has_upper && has_lower && dense
}

/// Pulls a `const NAME: T = "value";` name/value pair out of one line.
fn parse_const(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start().strip_prefix("const ")?;
    let (raw_name, rest) = trimmed.split_once(':')?;
    let name = raw_name.trim();
    if name.is_empty() || !name.chars().all(is_ident_char) {
        return None;
    }
    let value = const_value(rest)?;
    if value.len() < MIN_VALUE_LEN {
        return None;
    }
    Some((name.to_owned(), value))
}

/// Extracts the first `"..."` literal value from the right side of a const.
fn const_value(rest: &str) -> Option<String> {
    let (_, after_eq) = rest.split_once('=')?;
    let open = after_eq.find('"')?;
    let body = after_eq.get(open.saturating_add(1)..)?;
    let close = body.find('"')?;
    body.get(..close).map(str::to_owned)
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
    let (before, after) = trimmed.split_once('/')?;
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
    Some(head.trim().to_owned())
}

/// Hand-written rust source files under `src/` (the gate's scan scope).
///
/// # Errors
/// Returns an error if the root or eligible source paths cannot be read.
#[inline]
pub fn source_files() -> io::Result<Vec<PathBuf>> {
    let root = fs::canonicalize(".")?;
    let paths = required(&root)?;
    Ok(paths
        .into_iter()
        .filter(|path| is_source(path, &root))
        .collect())
}

/// Whether a candidate is a Rust source file under the scan root's `src/`.
fn is_source(path: &Path, root: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "rs")
        && path
            .strip_prefix(root)
            .is_ok_and(|relative| relative.starts_with("src"))
}

/// Reads each source file's `(path, text)`, skipping unreadable entries.
///
/// # Errors
/// Returns an error if source file enumeration fails.
fn sources() -> io::Result<Vec<(String, String)>> {
    let paths = source_files()?;
    let mut out = Vec::new();
    for path in paths {
        if let Ok(text) = fs::read_to_string(&path) {
            out.push((path.display().to_string(), text));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
