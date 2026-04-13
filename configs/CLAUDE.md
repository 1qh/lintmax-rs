# Project

Run `cargo lintmax` to check. Run `cargo lintmax fix` to auto-fix.

## Code style rules

### Explicit returns everywhere

Every function, closure, and match arm must use explicit `return`.

```rust
fn foo() -> i32 {
    return 42;
}
.map(|val| return val + 1)
.filter(|val| return !val.is_empty())
```

### Doc comments on everything, no regular comments

- Every `fn`, `const`, `struct`, `enum`, variant, and field needs `///`
- Crate root needs `//!`
- No `//` comments allowed anywhere
- `# Panics` section required on any fn that can panic

### Discarding results: the `discard()` pattern

Multiple lints conflict on how to ignore return values. Define this helper and use it everywhere:

```rust
fn discard<T>(_value: T) {}

discard(fs::write("file.txt", "content"));
discard(cmd("cargo", &["fmt"]));
```

### Alphabetical ordering

All items in a module must be alphabetically ordered: consts, then enums/structs, then fns. Enum variants must also be alphabetical.

### No absolute paths

Import with `use` first, never `std::io::stderr()` inline.

```rust
use std::io;
io::stderr()
```

### No raw string literals unless needed

Use regular string literals. `r#"..."#` only when the string contains `"` or `\`.

### Imports: one per line, grouped

```rust
use std::fs;
use std::io;
use std::path::Path;

use clap::Parser;
```

Standard library first, then external crates. One `use` per import.

### `#[cfg]` blocks and semicolons

Omit semicolon inside cfg block, put it outside:

```rust
#[cfg(unix)]
{
    discard(some_call())
};
```

### Single-character identifiers

Single-char names like `f`, `i`, `x` are allowed. But max 1 single-char binding in scope at a time. Use descriptive names: `status` not `s`, `command` not `c`.

### `as` casts forbidden

Use `From`, `Into`, `TryFrom`, `TryInto`:

```rust
ExitCode::from(u8::try_from(code).unwrap_or(1))
```

### `?` operator forbidden

Use explicit `match`, `if let`, `map`/`and_then`, or `discard()`.

### `unwrap()` and `expect()` forbidden

Use `unwrap_or`, `unwrap_or_default`, `unwrap_or_else`, or explicit matching.

### Print forbidden

No `println!`/`eprintln!`. Use `writeln!(io::stderr(), ...)` wrapped in `discard()`.

### `#[allow]` forbidden

Use `#[expect(lint, reason = "...")]` for documented exceptions. Never `#[allow]`.

## Lint severity

- `forbid`: all rustc allow-by-default lints. Cannot override.
- `deny`: clippy groups (pedantic, nursery, cargo, restriction). Can override with `#[expect]`.
- Three rustc lints use `deny` not `forbid`: `warnings`, `unused_extern_crates`, `unused_qualifications` (serde derive compatibility).

## Contradicting lint pairs (allowed)

| Allowed                            | Enforced                                       |
| ---------------------------------- | ---------------------------------------------- |
| `separated_literal_suffix`         | `unseparated_literal_suffix` — write `1i32`    |
| `pub_with_shorthand`               | `pub_without_shorthand` — write `pub(crate)`   |
| `self_named_module_files`          | `mod_module_files`                             |
| `exhaustive_enums/structs`         | non-exhaustive checking                        |
| `ref_patterns`                     | `pattern_type_mismatch` (also allowed)         |
| `needless_return`                  | `implicit_return` — explicit returns required  |
| `blanket_clippy_restriction_lints` | meta-lint                                      |
| `single_call_fn`                   | too noisy for real code                        |
| `semicolon_outside_block`          | conflicts with `semicolon_if_nothing_returned` |
| `semicolon_if_nothing_returned`    | conflicts with `semicolon_outside_block`       |
| `pattern_type_mismatch`            | too noisy for `for` loops                      |
