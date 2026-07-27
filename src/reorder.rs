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

/// Sorts every rust file under a root.
#[inline]
pub fn sort_tree(root: &Path) {
    for path in rust_files(root) {
        let _sorted = sort_file(&path);
    }
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
        for (open, close) in [('{', '}'), ('[', ']'), ('(', ')')] {
            let opens = isize::try_from(line.matches(open).count()).unwrap_or(0);
            let closes = isize::try_from(line.matches(close).count()).unwrap_or(0);
            depth = depth.saturating_add(opens).saturating_sub(closes);
            opened = opened || line.contains(open);
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
