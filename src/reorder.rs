//! Sorts declarations into the order the ordering lint declares, mechanically.
//!
//! The lint reports module-item grouping, impl-member order and type-member
//! order, and every instance is a permutation rather than a judgement. Each
//! rewrite is guarded by a token multiset compared with punctuation excluded,
//! because moving an item moves its braces and commas with it.

use core::ops::Range;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// The group a top-level item belongs to, in the order the lint declares.
const GROUPS: [(&str, u8); 15] = [
    ("const fn ", 5),
    ("async fn ", 5),
    ("unsafe fn ", 5),
    ("mod ", 0),
    ("use ", 1),
    ("macro_rules!", 2),
    ("const ", 3),
    ("static ", 3),
    ("type ", 4),
    ("enum ", 4),
    ("struct ", 4),
    ("union ", 4),
    ("trait ", 4),
    ("impl ", 4),
    ("fn ", 5),
];

/// One top-level item with the lines it spans.
struct Item {
    /// Last line index the item spans.
    end: usize,
    /// The group the item sorts into, or `None` for an item held in place.
    group: Option<u8>,
    /// First line index the item spans, including its docs and attributes.
    start: usize,
}

/// Reads the name a declaration carries, when its line opens one.
type NameOf = dyn Fn(&str) -> Option<String>;

/// One member of a block, with the lines it spans and the name it sorts by.
struct Member {
    /// Last line index the member spans.
    end: usize,
    /// Name the member sorts by, or `None` for a trailing run that holds.
    name: Option<String>,
    /// First line index the member spans, docs and attributes included.
    start: usize,
}

/// Whether the quote at `at` opens a character literal rather than a lifetime.
fn opens_char_literal(chars: &[char], at: usize) -> bool {
    let escaped = chars.get(at.saturating_add(1)) == Some(&'\\');
    let span = if escaped { 5_usize } else { 3_usize };
    let mut ahead = at.saturating_add(2);
    while ahead < at.saturating_add(span) {
        if chars.get(ahead) == Some(&'\'') {
            return true;
        }
        ahead = ahead.saturating_add(1);
    }
    return false;
}

/// The line with its string literals, character literals and comment removed.
fn code_only(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut kept = String::new();
    let mut index = 0_usize;
    while index < chars.len() {
        let Some(&character) = chars.get(index) else {
            break;
        };
        if character == '/' && chars.get(index.saturating_add(1)) == Some(&'/') {
            break;
        }
        if character == '"' || (character == '\'' && opens_char_literal(&chars, index)) {
            index = literal_end(&chars, index, character);
            continue;
        }
        kept.push(character);
        index = index.saturating_add(1);
    }
    return kept;
}

/// The index just past the literal opened at `at` by `quote`.
fn literal_end(chars: &[char], at: usize, quote: char) -> usize {
    let mut index = at.saturating_add(1);
    while index < chars.len() {
        match chars.get(index) {
            Some(&'\\') => index = index.saturating_add(2),
            Some(&found) if found == quote => return index.saturating_add(1),
            _ => index = index.saturating_add(1),
        }
    }
    return chars.len();
}

/// Sorts every rust file under a root: module items, then members.
#[inline]
pub fn sort_tree(root: &Path) {
    for path in rust_files(root) {
        let _sorted = sort_file(&path);
        let _members = sort_members(&path);
    }
}

/// The name a member declaration carries, when the line opens one.
fn member_name(line: &str) -> Option<String> {
    let bare = line.trim_start();
    let stripped = bare
        .split_once("pub(crate) ")
        .map_or(bare, |rest| return rest.1)
        .trim_start_matches("pub ");
    for keyword in [
        "fn ",
        "const fn ",
        "async fn ",
        "unsafe fn ",
        "const ",
        "type ",
    ] {
        let named = stripped.strip_prefix(keyword).map(leading_identifier);
        if let Some(found) = named.filter(|taken| return !taken.is_empty()) {
            return Some(found);
        }
    }
    return None;
}

/// The identifier a fragment opens with, which may be empty.
fn leading_identifier(fragment: &str) -> String {
    return fragment
        .chars()
        .take_while(|character| return character.is_alphanumeric() || *character == '_')
        .collect();
}

/// The name a struct field or enum variant carries, when the line declares one.
fn field_name(line: &str) -> Option<String> {
    let bare = line.trim_start();
    if bare.starts_with("//") || bare.starts_with('#') || bare.is_empty() {
        return None;
    }
    let stripped = bare
        .split_once("pub(crate) ")
        .map_or(bare, |rest| return rest.1)
        .trim_start_matches("pub ");
    let named = leading_identifier(stripped);
    if named.is_empty() {
        return None;
    }
    return Some(named);
}

/// Whether a line opens a top-level item, and which group it sorts into.
fn group_of(line: &str) -> Option<u8> {
    let bare = line
        .trim_start_matches("pub ")
        .trim_start_matches(|character: char| return character != ' ' && line.starts_with("pub("))
        .trim_start();
    let head = if bare.starts_with("pub(") {
        bare.split_once(british_close())
            .map_or(bare, |rest| return rest.1.trim_start())
    } else {
        bare
    };
    for &(prefix, group) in &GROUPS {
        if head.starts_with(prefix) || head.starts_with(&format!("async {prefix}")) {
            return Some(group);
        }
    }
    return None;
}

/// The character closing a visibility qualifier.
const fn british_close() -> char {
    return ')';
}

/// The items a file declares at top level, docs and attributes attached.
fn items(lines: &[String]) -> Vec<Item> {
    let mut found = Vec::new();
    let mut header: Option<usize> = None;
    let mut index = 0_usize;
    while index < lines.len() {
        let Some(line) = lines.get(index) else {
            break;
        };
        if line.starts_with("///") || line.starts_with("#[") {
            header = header.or(Some(index));
            index = index.saturating_add(1);
            continue;
        }
        let Some(group) = group_of(line) else {
            header = None;
            index = index.saturating_add(1);
            continue;
        };
        let start = header.unwrap_or(index);
        let gated = lines
            .get(start..=index)
            .is_some_and(|span| return span.iter().any(|held| return held.starts_with("#[cfg")));
        let end = extent(lines, index);
        found.push(Item {
            end,
            group: if gated { None } else { Some(group) },
            start,
        });
        header = None;
        index = end.saturating_add(1);
    }
    return found;
}

/// The last line of the item opening at `start`, counting every bracket kind.
fn extent(lines: &[String], start: usize) -> usize {
    let mut depth = 0_isize;
    let mut opened = false;
    let mut index = start;
    while index < lines.len() {
        let Some(line) = lines.get(index) else {
            break;
        };
        let code = code_only(line);
        for (open, close) in [('{', '}'), ('[', ']'), ('(', ')')] {
            let opens = isize::try_from(code.matches(open).count()).unwrap_or(0);
            let closes = isize::try_from(code.matches(close).count()).unwrap_or(0);
            depth = depth.saturating_add(opens).saturating_sub(closes);
            opened = opened || code.contains(open);
        }
        if depth <= 0 && (line.trim_end().ends_with(';') || opened) {
            return index;
        }
        index = index.saturating_add(1);
    }
    return lines.len().saturating_sub(1);
}

/// Every rust file under a root, skipping build output.
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(root) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ignored = path.file_name().is_some_and(|name| return name == "target");
        if path.is_dir() && !ignored {
            found.extend(rust_files(&path));
            continue;
        }
        if path.is_dir() {
            continue;
        }
        if path
            .extension()
            .is_some_and(|extension| return extension == "rs")
        {
            found.push(path);
        }
    }
    return found;
}

/// The lines a sorted file carries, or `None` when the order already ascends.
fn placed_lines(lines: &[String], found: &[Item]) -> Option<Vec<String>> {
    let groups: Vec<u8> = found.iter().filter_map(|item| return item.group).collect();
    let mut sorted = groups.clone();
    sorted.sort_unstable();
    if groups == sorted {
        return None;
    }
    let mut movable: Vec<&Item> = found
        .iter()
        .filter(|item| return item.group.is_some())
        .collect();
    movable.sort_by_key(|item| return (item.group, item.start));
    let mut order = movable.into_iter();
    let mut rebuilt: Vec<String> = Vec::new();
    let mut index = 0_usize;
    for item in found {
        carry(&mut rebuilt, lines, index..item.start);
        let placed = match item.group {
            None => item,
            Some(_) => match order.next() {
                Some(next) => next,
                None => return None,
            },
        };
        carry(
            &mut rebuilt,
            lines,
            placed.start..placed.end.saturating_add(1),
        );
        index = item.end.saturating_add(1);
    }
    carry(&mut rebuilt, lines, index..lines.len());
    return Some(rebuilt);
}

/// Copies a span of lines onto the rebuilt file.
fn carry(rebuilt: &mut Vec<String>, lines: &[String], span: Range<usize>) {
    if let Some(taken) = lines.get(span) {
        rebuilt.extend(taken.iter().cloned());
    }
}

/// Sorts one file's top-level items, refusing any rewrite that loses a token.
fn sort_file(path: &Path) -> bool {
    let Ok(original) = fs::read_to_string(path) else {
        return false;
    };
    let lines: Vec<String> = original.split('\n').map(str::to_owned).collect();
    let Some(rebuilt) = placed_lines(&lines, &items(&lines)) else {
        return false;
    };
    let rewritten = rebuilt.join("\n");
    if tokens(&rewritten) != tokens(&original) {
        return false;
    }
    return fs::write(path, rewritten).is_ok();
}

/// The members a block declares between `open` and `close`.
fn members(lines: &[String], open: usize, close: usize, named: &NameOf) -> Vec<Member> {
    let mut found: Vec<Member> = Vec::new();
    let mut header: Option<usize> = None;
    let mut index = open.saturating_add(1);
    while index < close {
        let Some(line) = lines.get(index) else {
            break;
        };
        let trimmed = line.trim_start();
        let is_header = trimmed.starts_with("///") || trimmed.starts_with("#[");
        if is_header {
            header = header.or(Some(index));
            index = index.saturating_add(1);
            continue;
        }
        if trimmed.is_empty() {
            index = index.saturating_add(1);
            continue;
        }
        let Some(name) = named(line) else {
            header = None;
            index = index.saturating_add(1);
            continue;
        };
        let start = header.unwrap_or(index);
        let end = member_extent(lines, index, close);
        found.push(Member {
            end,
            name: Some(name),
            start,
        });
        header = None;
        index = end.saturating_add(1);
    }
    return found;
}

/// The last line of the member opening at `start`, bounded by its block.
fn member_extent(lines: &[String], start: usize, close: usize) -> usize {
    let mut depth = 0_isize;
    let mut index = start;
    while index < close {
        let Some(line) = lines.get(index) else {
            break;
        };
        let code = code_only(line);
        let opens = isize::try_from(code.matches('{').count()).unwrap_or(0);
        let closes = isize::try_from(code.matches('}').count()).unwrap_or(0);
        depth = depth.saturating_add(opens).saturating_sub(closes);
        let trimmed = line.trim_end();
        if depth <= 0
            && (trimmed.ends_with('}') || trimmed.ends_with(';') || trimmed.ends_with(','))
        {
            return index;
        }
        index = index.saturating_add(1);
    }
    return close.saturating_sub(1);
}

/// The line closing the block opened at `open`.
fn block_close(lines: &[String], open: usize) -> usize {
    let mut depth = 0_isize;
    let mut index = open;
    while index < lines.len() {
        let Some(line) = lines.get(index) else {
            break;
        };
        let code = code_only(line);
        let opens = isize::try_from(code.matches('{').count()).unwrap_or(0);
        let closes = isize::try_from(code.matches('}').count()).unwrap_or(0);
        depth = depth.saturating_add(opens).saturating_sub(closes);
        if depth <= 0 && index > open {
            return index;
        }
        index = index.saturating_add(1);
    }
    return lines.len().saturating_sub(1);
}

/// Sorts every block's members in a file's lines, reporting whether any moved.
fn sorted_blocks(lines: &mut Vec<String>) -> bool {
    let mut changed = false;
    let mut index = 0_usize;
    while index < lines.len() {
        let Some(line) = lines.get(index) else {
            break;
        };
        let Some(named) = reader_for(line) else {
            index = index.saturating_add(1);
            continue;
        };
        let close = block_close(lines, index);
        if sort_block(lines, index, close, named) {
            changed = true;
        }
        index = close.saturating_add(1);
    }
    return changed;
}

/// Sorts the members of every impl block and every type in one file.
fn sort_members(path: &Path) -> bool {
    let Ok(original) = fs::read_to_string(path) else {
        return false;
    };
    let mut lines: Vec<String> = original.split('\n').map(str::to_owned).collect();
    if !sorted_blocks(&mut lines) {
        return false;
    }
    let rewritten = lines.join("\n");
    if tokens(&rewritten) != tokens(&original) {
        return false;
    }
    return fs::write(path, rewritten).is_ok();
}

/// The name reader a block's opening line calls for, if it opens one at all.
fn reader_for(line: &'_ str) -> Option<&'static NameOf> {
    if !line.trim_end().ends_with('{') {
        return None;
    }
    if line.starts_with("impl ") || line.starts_with("impl<") {
        return Some(&member_name);
    }
    if opens_type(line) {
        return Some(&field_name);
    }
    return None;
}

/// Whether a line opens a struct or enum whose members sort by name.
fn opens_type(line: &str) -> bool {
    let bare = line
        .split_once("pub(crate) ")
        .map_or(line, |rest| return rest.1)
        .trim_start_matches("pub ");
    return bare.starts_with("struct ") || bare.starts_with("enum ");
}

/// Sorts one block's members in place, reporting whether anything moved.
fn sort_block(lines: &mut Vec<String>, open: usize, close: usize, named: &NameOf) -> bool {
    let found = members(lines, open, close, named);
    let order: Vec<String> = found
        .iter()
        .filter_map(|item| return item.name.clone())
        .collect();
    let mut sorted = order.clone();
    sorted.sort();
    if order == sorted || order.len() < 2 {
        return false;
    }
    let mut ranked: Vec<&Member> = found.iter().collect();
    ranked.sort_by(|left, right| return left.name.cmp(&right.name));
    let mut rebuilt: Vec<String> = Vec::new();
    for member in ranked {
        if let Some(span) = lines.get(member.start..=member.end) {
            rebuilt.extend(span.iter().cloned());
        }
    }
    let Some(first) = found.first() else {
        return false;
    };
    let Some(last) = found.last() else {
        return false;
    };
    drop(lines.splice(first.start..=last.end, rebuilt));
    return true;
}

/// The identifiers and numbers a source carries, sorted, punctuation excluded.
fn tokens(source: &str) -> Vec<String> {
    let mut found: Vec<String> = source
        .split(|character: char| return !character.is_alphanumeric() && character != '_')
        .filter(|piece| return !piece.is_empty())
        .map(str::to_owned)
        .collect();
    found.sort_unstable();
    return found;
}

#[cfg(test)]
mod tests;
