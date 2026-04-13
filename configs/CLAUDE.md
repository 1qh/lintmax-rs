# Project

Run `cargo lintmax fix` after every change.

## Rules — do this, NOT that

### Returns

```rust
// WRONG
fn foo() -> i32 { 42 }
// RIGHT
fn foo() -> i32 { return 42; }

// WRONG
.map(|val| val + 1)
// RIGHT
.map(|val| return val + 1)
```

### Doc comments

```rust
// WRONG — regular comment
// this adds two numbers
fn add(left: i32, right: i32) -> i32 { return left + right; }

// WRONG — missing doc
fn add(left: i32, right: i32) -> i32 { return left + right; }

// RIGHT
/// Adds two numbers.
fn add(left: i32, right: i32) -> i32 { return left + right; }

// WRONG — missing crate doc
use std::fs;

// RIGHT
//! My crate description.
use std::fs;
```

### Discarding results

```rust
// WRONG
fs::write("f.txt", "x").ok();
// WRONG
let _ = fs::write("f.txt", "x");
// WRONG
drop(fs::write("f.txt", "x"));

// RIGHT — define once, use everywhere
fn discard<T>(_value: T) {}
discard(fs::write("f.txt", "x"));
```

### Ordering

```rust
// WRONG
fn zebra() {}
fn alpha() {}
const Z: i32 = 1;
const A: i32 = 0;

// RIGHT — consts first (alphabetical), then fns (alphabetical)
const A: i32 = 0;
const Z: i32 = 1;
fn alpha() {}
fn zebra() {}
```

### Paths

```rust
// WRONG
std::io::stderr()

// RIGHT
use std::io;
io::stderr()
```

### Imports

```rust
// WRONG
use std::{fs, io};
use clap::{Parser, Subcommand};

// RIGHT
use std::fs;
use std::io;

use clap::Parser;
use clap::Subcommand;
```

### Casts

```rust
// WRONG
let byte = code as u8;

// RIGHT
let byte = u8::try_from(code).unwrap_or(1);
```

### Error handling

```rust
// WRONG
let val = something()?;
// WRONG
let val = something().unwrap();
// WRONG
let val = something().expect("msg");

// RIGHT
let val = something().unwrap_or_default();
// RIGHT
let val = match something() {
    Ok(inner) => inner,
    Err(_) => return ExitCode::FAILURE,
};
```

### Printing

```rust
// WRONG
println!("hello");
// WRONG
eprintln!("error");

// RIGHT
use std::io;
use std::io::Write as _;
discard(writeln!(io::stderr(), "error"));
```

### Suppressing lints

```rust
// WRONG
#[allow(clippy::some_lint)]

// RIGHT
#[expect(clippy::some_lint, reason = "explanation why this is needed")]
```

### Literal suffixes

```rust
// WRONG
let x = 1_i32;
// RIGHT
let x = 1i32;
```

### `#[cfg]` blocks

```rust
// WRONG
#[cfg(unix)]
{
    discard(some_call());
}

// RIGHT
#[cfg(unix)]
{
    discard(some_call())
};
```

### Raw strings

```rust
// WRONG
let s = r#"hello world"#;
// RIGHT
let s = "hello world";

// OK — needs raw string because of quotes
let s = r#"say "hello""#;
```

### Variable names

```rust
// WRONG — 2+ single-char names in scope
let s = get_status();
let c = get_command();

// RIGHT
let status = get_status();
let command = get_command();

// OK — single single-char name
let f: &mut fmt::Formatter<'_>
```

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
