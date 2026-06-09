//! Span-aware `//` comment detection and stripping for rust source lines.

/// Cursor over a line's chars; centralizes peeking so the scanner never indexes.
struct Cursor {
    /// The line decomposed into chars.
    chars: Vec<char>,
    /// Next index to read.
    pos: usize,
}

/// Lexer mode while scanning a rust source line for comments.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scan {
    /// Inside a `'.'` char literal.
    Char,
    /// Ordinary code.
    Code,
    /// Inside a raw string with the given number of `#` hashes.
    Raw(usize),
    /// Inside a `"..."` string literal.
    Str,
}

/// Outcome of consuming one char in code mode.
enum Step {
    /// Stop scanning with this rewritten line and removed flag.
    Done(String, bool),
    /// Continue scanning in the given mode.
    Next(Scan),
}

impl Cursor {
    /// Char at an absolute offset from the current position.
    fn at(&self, ahead: usize) -> Option<char> {
        let chars = &self.chars;
        let found = self
            .pos
            .checked_add(ahead)
            .and_then(|idx| return chars.get(idx).copied());
        return found;
    }

    /// Prefix of the line up to the current position, trailing space trimmed.
    fn code_before(&self) -> String {
        let slice = self.chars.get(..self.pos).unwrap_or(&self.chars);
        return slice.iter().collect::<String>().trim_end().to_owned();
    }

    /// Whole line as a string.
    fn line(&self) -> String {
        return self.chars.iter().collect();
    }

    /// Builds a cursor over a line.
    fn new(line: &str) -> Self {
        return Self {
            chars: line.chars().collect(),
            pos: 0,
        };
    }

    /// Advances the cursor by one char.
    const fn step(&mut self) {
        self.pos = self.pos.saturating_add(1);
    }

    /// Advances the cursor by two chars (consume char plus its escape).
    const fn step_escape(&mut self) {
        self.pos = self.pos.saturating_add(2);
    }
}

/// Whether the cursor sits on a raw-string terminator for `hashes` hashes.
fn closes_raw(cursor: &Cursor, hashes: usize) -> bool {
    for offset in 1..=hashes {
        if cursor.at(offset) != Some('#') {
            return false;
        }
    }
    return true;
}

/// Other literal mode for a closing quote (`"` → `Str`, else `Char`).
const fn quoted_mode(close: char) -> Scan {
    if close == '"' {
        return Scan::Str;
    }
    return Scan::Char;
}

/// Whether the char at the cursor opens a char literal (`'x'`, `'\n'`) rather
/// than a lifetime (`'a`), so lifetimes never flip the scanner into char mode.
fn opens_char_literal(cursor: &Cursor) -> bool {
    return cursor.at(1) == Some('\\') || cursor.at(2) == Some('\'');
}

/// Whether the next chars open a survivor doc comment (`///`, `//!`).
fn opens_doc_comment(cursor: &Cursor) -> bool {
    return matches!(cursor.at(2), Some('/' | '!'));
}

/// Counts the `#` run after `r`, returning the count when a `"` follows.
fn raw_string_hashes(cursor: &Cursor) -> Option<usize> {
    let mut hashes: usize = 0;
    while cursor.at(hashes.saturating_add(1)) == Some('#') {
        hashes = hashes.saturating_add(1);
    }
    if cursor.at(hashes.saturating_add(1)) == Some('"') {
        return Some(hashes);
    }
    return None;
}

/// Consumes one char in code mode, returning the scan step.
fn step_code(cursor: &mut Cursor, cur: char) -> Step {
    if cur == '/' && cursor.at(1) == Some('/') {
        if opens_doc_comment(cursor) {
            return Step::Done(cursor.line(), false);
        }
        return Step::Done(cursor.code_before(), true);
    }
    if cur == 'r'
        && let Some(hashes) = raw_string_hashes(cursor)
    {
        for _ in 0..hashes.saturating_add(2) {
            cursor.step();
        }
        return Step::Next(Scan::Raw(hashes));
    }
    cursor.step();
    if cur == '"' {
        return Step::Next(Scan::Str);
    }
    if cur == '\'' && opens_char_literal(cursor) {
        return Step::Next(Scan::Char);
    }
    return Step::Next(Scan::Code);
}

/// Consumes one char inside a `"..."` or `'.'` literal, returning the next mode.
const fn step_quoted(cursor: &mut Cursor, cur: char, close: char) -> Scan {
    if cur == '\\' {
        cursor.step_escape();
        return quoted_mode(close);
    }
    cursor.step();
    if cur == close {
        return Scan::Code;
    }
    return quoted_mode(close);
}

/// Consumes one char inside a raw string, returning the next mode.
fn step_raw(cursor: &mut Cursor, cur: char, hashes: usize) -> Scan {
    if cur == '"' && closes_raw(cursor, hashes) {
        for _ in 0..=hashes {
            cursor.step();
        }
        return Scan::Code;
    }
    cursor.step();
    return Scan::Raw(hashes);
}

/// Strips a non-survivor `//` comment from one line. Preserves code, strings,
/// char literals, raw strings, and survivors (`///`, `//!`); returns the
/// rewritten line and whether a comment was removed.
#[inline]
#[must_use]
pub fn strip_line(line: &str) -> (String, bool) {
    let mut cursor = Cursor::new(line);
    let mut state = Scan::Code;
    while let Some(cur) = cursor.at(0) {
        state = match state {
            Scan::Char => step_quoted(&mut cursor, cur, '\''),
            Scan::Code => match step_code(&mut cursor, cur) {
                Step::Done(result, removed) => return (result, removed),
                Step::Next(next) => next,
            },
            Scan::Raw(hashes) => step_raw(&mut cursor, cur, hashes),
            Scan::Str => step_quoted(&mut cursor, cur, '"'),
        };
    }
    return (line.to_owned(), false);
}

#[cfg(test)]
mod tests {
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
}
