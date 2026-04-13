//! Copies `configs/CLAUDE.md` to project root so Claude Code can read it.

use std::fs;

/// Discards a value.
fn discard<T>(_value: T) {}

/// Entry point.
fn main() {
    let source = fs::read_to_string("configs/CLAUDE.md").unwrap_or_default();
    discard(fs::write("CLAUDE.md", source));
}
