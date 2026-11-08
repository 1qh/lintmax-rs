# lintmax

Maximum strictness Rust pipeline in one command. Zero config in your project.

## Install

```sh
cargo install cargo-lintmax
```

## Usage — exactly four commands

```sh
cargo lintmax          # default = fix: format + autofix + the full gate
cargo lintmax fix      # same as the default
cargo lintmax check    # CI verify: read-only full gate, no writes
cargo lintmax version  # print the version
cargo lintmax rules    # list the active rule set
```

Everything else the tool does for itself, never as a command: toolchain `@latest` refresh on cadence (forced under CI), the green-tree-hash cache, and the dependency-staleness scan.

## What it does

One command runs, over every applicable file at max strictness:

- clippy — every lint group (pedantic, nursery, cargo, restriction) denied, `--all-targets --all-features` (lib + bins + tests + examples + benches)
- rustfmt over all `.rs`
- dprint over every other file type (toml, json, md, yaml, dockerfile, css, html)
- shellcheck + shfmt over every shell script
- typos over the whole repo
- no-`//`-comment check + comment strip (only `///` and `//!` survive)
- doc build with warnings denied
- cargo-nextest tests + doc tests
- cargo-deny dependency audit + cargo-machete unused-dep check
- in-house advisories: dupconst, gibberish-identifier, unguarded-float-division

All configs are embedded in the binary. Your project stays clean — no clippy.toml, no rustfmt.toml, no deny.toml, no dprint.json. Update `cargo-lintmax` = update every project's strictness.

## What's enforced

- Every stable rustc allow-by-default lint: `forbid`
- Every clippy group (pedantic, nursery, cargo, restriction): `deny`
- Zero warnings: `warnings = deny`
- No `//` comments (only `///` doc comments)
- Formatting via `cargo fmt` + `dprint` + `shfmt`
- Shell linting via `shellcheck` (every optional check on)
- Spell checking via `typos`
- Dependency audit via `cargo deny`
- Unused dependencies via `cargo machete`
- Doc warnings denied

## Exceptions

Declare project exceptions in `lintmax-exceptions.toml` at the project root.
The `advisories`, `duplicates`, and `generated` lists name advisory identifiers,
`crate@version` duplicates, and generator-owned paths, respectively.
Generated paths are excluded from dprint, typos (check and fix), and the staleness scan.
The `words` list declares project vocabulary: each word is accepted unchanged by
typos through `extend-words`, while every other word is still checked.
Set `LINTMAX_EXCEPTIONS` to an absolute path anywhere on disk to load an additional
exceptions file. When both files exist, all four lists are merged. An unreadable
or invalid explicitly selected file fails the gate with its path, including when
the green cache would otherwise match.
