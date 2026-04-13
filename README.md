# lintmax

Maximum strictness Rust pipeline in one command. Zero config in your project.

## Install

```sh
cargo install cargo-lintmax
```

## Usage

```sh
cargo lintmax          # run full check pipeline
cargo lintmax ci       # clean + update + check all
cargo lintmax ci-remote # update + check all (for CI)
cargo lintmax fix      # auto-fix everything
cargo lintmax fmt      # format all files
cargo lintmax watch    # dev loop with bacon
cargo lintmax cov      # coverage report
cargo lintmax sync     # sync hooks, CI, gitignore, CLAUDE.md
```

## What it does

One command runs: format check, spell check, no-comments check, clippy (every lint group at max severity), tests, dependency audit, unused dep check, doc build with warnings denied.

All configs are embedded in the binary. Your project stays clean — no clippy.toml, no rustfmt.toml, no deny.toml, no dprint.json. Update `cargo-lintmax` = update every project's strictness.

## What's enforced

- Every stable rustc allow-by-default lint: `forbid`
- Every clippy group (pedantic, nursery, cargo, restriction): `deny`
- Zero warnings: `warnings = deny`
- No `//` comments (only `///` doc comments)
- Formatting via `cargo fmt` + `dprint`
- Spell checking via `typos`
- Dependency audit via `cargo deny`
- Unused dependencies via `cargo machete`
- Doc warnings denied
