# lintmax-rs

See [README.md](README.md) for usage.

## Architecture

Single binary (`cargo-lintmax`) that embeds all config files and lint flags. No config files needed in target projects. Configs are written as temp files before checks and cleaned up after. Lint flags are passed via CLI args to `cargo clippy`, not via `Cargo.toml`.

## Code style rules (enforced by the tool on itself)

### Implicit returns

`needless_return` (style) is enforced: a function, closure or match arm ends in its value, never in `return`. The restriction lint `implicit_return` demands the opposite and is allowed, because a restriction lint that contradicts a default or pedantic lint gives way to it, so code that passes this gate also passes a project's standard `cargo clippy -D warnings`.

```rust
fn foo() -> i32 {
    42
}
.map(|val| val + 1)
.filter(|val| !val.is_empty())
```

### Doc comments on everything, no regular comments

- `missing_docs` is `forbid` (rustc) and `missing_docs_in_private_items` is active (clippy restriction)
- Every `fn`, `const`, `struct`, `enum`, variant, and field needs `///`
- Crate root needs `//!`
- No `//` comments allowed anywhere, line-leading OR trailing-inline — a span-aware scanner (`src/comment.rs`) flags/strips them while preserving `///`, `//!`, and `//` inside strings/char/raw-string literals
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

`semicolon_outside_block` (restriction) and `semicolon_if_nothing_returned` (pedantic) conflict on `#[cfg]` blocks. The pedantic lint is enforced and the restriction lint is allowed, so the semicolon goes inside the block:

```rust
#[cfg(unix)]
{
    discard(some_call());
}
```

### Single-character identifiers

`min_ident_chars` is active with threshold 1. Single-char names like `f`, `i`, `x` are configured as allowed exceptions in `clippy.toml`. But `many_single_char_names` (pedantic) fires if more than 1 single-char binding is in scope simultaneously. Use descriptive names: `status` not `s`, `command` not `c`, `first`/`second` not `a`/`b`.

### Pattern matching on references

`pattern_type_mismatch` is enforced and `ref_patterns` is allowed. When iterating over `&[(&str, &str)]`, use `for &(key, val)` to destructure the reference explicitly.

### `as` casts forbidden

`as_conversions` (restriction) forbids `as` casts. Use `From`, `Into`, `TryFrom`, `TryInto` instead:

```rust
ExitCode::from(u8::try_from(code).unwrap_or(1))
```

### `?` for error propagation

`question_mark` (style) is enforced: propagate an error with `?`, never with a `match` that only returns the error. The restriction lint `question_mark_used` forbids `?` and is allowed, for the same reason as `implicit_return`.

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

These allows resolve conflicts with enforced lints or the selected module layout. Where a restriction lint contradicts a default or pedantic lint, the default or pedantic lint is enforced, so code passing this gate also passes a project's standard clippy:

| Allowed                    | Kept (enforced)                                                              |
| -------------------------- | ---------------------------------------------------------------------------- |
| `separated_literal_suffix` | `unseparated_literal_suffix` — write `1i32` not `1_i32`                      |
| `pub_with_shorthand`       | `pub_without_shorthand` — write `pub(crate)` not `pub`                       |
| `self_named_module_files`  | `mod_module_files` — use `mod.rs` for directory modules                      |
| `redundant_pub_crate`      | `pub_without_shorthand` — retain explicit crate visibility in binary crates  |
| `ref_patterns`             | `pattern_type_mismatch` — permit explicit reference patterns                 |
| `implicit_return`          | `needless_return` — end in the value, never in `return`                      |
| `question_mark_used`       | `question_mark` — propagate errors with `?`                                  |
| `semicolon_outside_block`  | `semicolon_if_nothing_returned` — the semicolon goes inside a `#[cfg]` block |

### Additionally allowed for practicality

| Allowed                            | Reason                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cognitive_complexity`             | Counts code expanded from macros (rust-lang/rust-clippy #14417, open): a function holding only one `tracing::warn!` scores 8 against a threshold of 5 once `tracing`'s `log` feature is on, which `sqlx-core` enables unconditionally, so it judges the macro, not the function. `too_many_lines`, `excessive_nesting`, `too_many_arguments` and the bool-parameter lints still bound each function. Revisit when #14417 closes |
| `single_call_fn`                   | Flags any fn called once — would require inlining all dispatch logic                                                                                                                                                                                                                                                                                                                                                            |
| `blanket_clippy_restriction_lints` | The gate deliberately enables the entire restriction group                                                                                                                                                                                                                                                                                                                                                                      |
| `float_arithmetic`                 | The floatdiv advisory checks unguarded float division more precisely                                                                                                                                                                                                                                                                                                                                                            |
| `multiple_crate_versions`          | The cargo-deny stage audits duplicates with declared exceptions                                                                                                                                                                                                                                                                                                                                                                 |

`exhaustive_enums`, `exhaustive_structs`, and `pattern_type_mismatch` remain enforced. Matching another crate's `#[non_exhaustive]` enum with a `_` arm is accepted by `wildcard_enum_match_arm`, so it does not contradict `exhaustive_enums`.

## Formatter options excluded for correctness

`format_strings` stays `false`. It is an unstable rustfmt option that rewrites string literals, and upstream documents that it can change what a literal says or produce invalid code: rust-lang/rustfmt #5138 (breaks after a backslash, invalid output), #4471 (a line break beside an escape changes the meaning), #4608 (trailing spaces removed from a literal over `max_width`). A consumer measured the first: a `\"`-escaped JSON literal split into code that no longer compiled. A formatter must preserve meaning, so a long literal stays a width finding for the author to split by hand. Revisit when those issues close.

## Licence audit scope

`cargo deny` audits the licence of every dependency against the allow list. A workspace crate that is never published (`publish = false`) is skipped (`[licenses.private] ignore = true`): its licence decides nothing about what the project may ship, and a private company crate has no allow-listed licence it could truthfully declare.

## Nightly/unstable lints excluded

These rustc lints exist but are nightly-only and will error on stable with "unknown lint":

- `deprecated_llvm_intrinsic`
- `implicit_provenance_casts`
- `multiple_supertrait_upcastable`
- `must_not_suspend`
- `non_exhaustive_omitted_patterns`
- `resolving_to_items_shadowing_supertrait_items`
- `shadowing_supertrait_items`
- `unqualified_local_imports`

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
- `rg` (ripgrep) — source file enumeration
- `curl` — resolving latest dprint plugin versions (never-stale)
- `bacon` — dev loop (watch mode)

## Config file behavior

- `write_configs()` writes temp files before checks, `clean_configs()` removes them after
- If a config file already exists with different content (user-customized), lintmax does NOT overwrite it
- If the file matches embedded content exactly, it is cleaned up (considered lintmax-managed)
- A dprint config is also lintmax-managed when every non-plugin setting matches the embedded config and the plugin set is unchanged. Plugin versions, ordering, and legacy URL versus npm specifier syntax may differ. Changed settings or added, removed, or replaced plugins make it user-customized and it is kept.
- The only files lintmax writes in a consumer are the temp linter configs above, removed after the run; it never scaffolds consumer project files

## Testing the tool

`cargo lintmax` must pass on the lintmax-rs repo itself. This is the dogfooding requirement. Any change to lint rules must also satisfy the tool's own source code.
