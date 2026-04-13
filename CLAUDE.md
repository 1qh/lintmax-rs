# lintmax-rs

See [README.md](README.md) for usage.

## Architecture

Single binary (`cargo-lintmax`) that embeds all config files and lint flags. No config files needed in target projects. Configs are written as temp files before checks and cleaned up after. Lint flags are passed via CLI args to `cargo clippy`, not via `Cargo.toml`.

## Code style rules (enforced by the tool on itself)

### Explicit returns everywhere

`implicit_return` (restriction) is active. Every function, closure, and match arm must use explicit `return`. This conflicts with `needless_return` (style), which is allowed.

```rust
fn foo() -> i32 {
    return 42;
}
.map(|val| return val + 1)
.filter(|val| return !val.is_empty())
```

### Doc comments on everything, no regular comments

- `missing_docs` is `forbid` (rustc) and `missing_docs_in_private_items` is active (clippy restriction)
- Every `fn`, `const`, `struct`, `enum`, variant, and field needs `///`
- Crate root needs `//!`
- No `//` comments allowed anywhere — enforced by `rg` check in pipeline
- `# Panics` section required on any fn that can panic (`missing_panics_doc` from pedantic)

### Discarding results: the `discard()` pattern

Multiple lints conflict on how to ignore return values:

| Pattern              | Lint that rejects it                                             |
| -------------------- | ---------------------------------------------------------------- |
| `.ok()`              | `unused_result_ok`                                               |
| `let _ = expr`       | `let_underscore_must_use`, `let_underscore_untyped`              |
| `let _: Type = expr` | `redundant_type_annotations` (if type is inferable)              |
| `drop(expr)`         | `dropping_copy_types` (if Copy), `dropping_references` (if &mut) |

Solution: `fn discard<T>(_value: T) {}` — a no-op generic function that consumes any value. Satisfies all lints simultaneously.

```rust
discard(fs::write("file.txt", "content"));
discard(cmd("cargo", &["fmt"]));
discard(command.args(["--flag"]));
```

### Alphabetical ordering

`arbitrary_source_item_ordering` (restriction) requires all items in a module to be alphabetically ordered: consts, then enums/structs, then fns. Within each category, alphabetical by name.

Enum variants must also be alphabetical.

### No absolute paths

`absolute_paths` (restriction) forbids `std::io::stderr()`. Must import first:

```rust
use std::io;
io::stderr()
```

### No raw string literals unless needed

`needless_raw_strings` (restriction) rejects `r#"..."#` when the string contains no characters that need escaping. Use regular string literals with escape sequences instead.

For multi-line strings with special characters, use string concatenation with `\n\` continuation.

### Imports: one per line, grouped

```rust
use std::fs;
use std::io;
use std::path::Path;

use clap::Parser;
```

Standard library first, then external crates. One `use` per import — no `{A, B}` grouping.

### `#[cfg]` blocks and semicolons

`semicolon_outside_block` (restriction) and `semicolon_if_nothing_returned` (pedantic) conflict on `#[cfg]` blocks. Both are allowed in the lint config. When writing cfg blocks that call `discard()`, omit the semicolon inside and put it outside:

```rust
#[cfg(unix)]
{
    discard(some_call())
};
```

### Single-character identifiers

`min_ident_chars` is active with threshold 1. Single-char names like `f`, `i`, `x` are configured as allowed exceptions in `clippy.toml`. But `many_single_char_names` (pedantic) fires if more than 1 single-char binding is in scope simultaneously. Use descriptive names: `status` not `s`, `command` not `c`, `first`/`second` not `a`/`b`.

### Pattern matching on references

`pattern_type_mismatch` and `ref_patterns` conflict. Both are in restriction. `ref_patterns` is allowed. When iterating over `&[(&str, &str)]`, use `for &(key, val)` or just `for (key, val)` — the `pattern_type_mismatch` lint is also allowed so either works.

### `as` casts forbidden

`as_conversions` (restriction) forbids `as` casts. Use `From`, `Into`, `TryFrom`, `TryInto` instead:

```rust
ExitCode::from(u8::try_from(code).unwrap_or(1))
```

### `?` operator forbidden

`question_mark_used` (restriction) is active. Cannot use `?` for error propagation. Must use explicit `match`, `if let`, or `map`/`and_then` chains. For infallible-in-practice operations, use `discard()`.

### `unwrap()` and `expect()` forbidden

`unwrap_used` and `expect_used` (restriction) are active. Use `unwrap_or`, `unwrap_or_default`, `unwrap_or_else`, or explicit matching.

### Print to stderr only, and even that is restricted

`print_stdout` and `print_stderr` (restriction) are both active. For necessary user-facing output, use `writeln!(io::stderr(), ...)` and wrap in `discard()`. Never use `println!`, `eprintln!`, `print!`, or `eprint!`.

## Lint severity levels

### `forbid` vs `deny`

- `forbid`: cannot be overridden even with `#[expect]`. Used for all rustc allow-by-default lints.
- `deny`: hard error but can be overridden with `#[expect(lint, reason = "...")]`. Used for clippy groups because `forbid` on clippy groups prevents `#[expect]` on any lint in the group, making code unwritable.
- Three rustc lints use `deny` instead of `forbid`: `warnings` (test harness injects `#[allow(dead_code)]` which conflicts with `forbid`), `unused_extern_crates` and `unused_qualifications` (serde derive macros emit `#[allow]` for these).
- `rust_2018_idioms` group uses `deny` because it includes `unused_extern_crates` which serde needs to override.

### Contradicting clippy lint pairs

These lints are in the restriction group and directly contradict each other. One side must be allowed:

| Allowed                            | Kept (enforced)                                              |
| ---------------------------------- | ------------------------------------------------------------ |
| `separated_literal_suffix`         | `unseparated_literal_suffix` — write `1i32` not `1_i32`      |
| `pub_with_shorthand`               | `pub_without_shorthand` — write `pub(crate)` not `pub`       |
| `self_named_module_files`          | `mod_module_files`                                           |
| `exhaustive_enums`                 | non-exhaustive checking                                      |
| `exhaustive_structs`               | non-exhaustive checking                                      |
| `ref_patterns`                     | `pattern_type_mismatch` (also allowed due to impracticality) |
| `needless_return`                  | `implicit_return` — explicit returns required                |
| `blanket_clippy_restriction_lints` | meta-lint about enabling the restriction group               |

### Additionally allowed for practicality

| Allowed                         | Reason                                                               |
| ------------------------------- | -------------------------------------------------------------------- |
| `single_call_fn`                | Flags any fn called once — would require inlining all dispatch logic |
| `semicolon_outside_block`       | Conflicts with `semicolon_if_nothing_returned` on `#[cfg]` blocks    |
| `semicolon_if_nothing_returned` | Same conflict, opposite direction                                    |
| `pattern_type_mismatch`         | Requires destructuring references in every `for` loop — too noisy    |

## Nightly/unstable lints excluded

These rustc lints exist but are nightly-only and will error on stable with "unknown lint":

- `fuzzy_provenance_casts`
- `lossy_provenance_casts`
- `multiple_supertrait_upcastable`
- `must_not_suspend`
- `non_exhaustive_omitted_patterns`
- `shadowing_supertrait_items`
- `unqualified_local_imports`
- `resolving_to_items_shadowing_supertrait_items`

Also excluded: `linker_messages` (fires on Homebrew toolchain noise, not code issues).

## External tool dependencies

The binary shells out to these tools (must be installed):

- `cargo fmt` (rustfmt) — Rust formatting
- `cargo clippy` — Rust linting
- `cargo nextest` — test runner
- `cargo deny` — dependency audit
- `cargo machete` — unused dependency detection
- `cargo llvm-cov` — coverage
- `dprint` — TOML/JSON/YAML/Markdown formatting
- `typos` — spell checking
- `rg` (ripgrep) — comment detection
- `perl` — comment removal in `fix` subcommand
- `bacon` — dev loop (watch mode)

## Config file behavior

- `write_configs()` writes temp files before checks, `clean_configs()` removes them after
- If a config file already exists with different content (user-customized), lintmax does NOT overwrite it
- If the file matches embedded content exactly, it is cleaned up (considered lintmax-managed)
- `cargo lintmax sync` writes managed files: `.githooks/pre-commit`, `.github/workflows/ci.yml`, `.gitignore`, `.editorconfig`, `rust-analyzer.toml`, `CLAUDE.md`

## Testing the tool

`cargo lintmax` must pass on the lintmax-rs repo itself. This is the dogfooding requirement. Any change to lint rules must also satisfy the tool's own source code.
