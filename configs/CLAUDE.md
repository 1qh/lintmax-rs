# Project

Run `cargo lintmax` to check. Run `cargo lintmax fix` to auto-fix.

## Rules

- Use `#[expect(lint, reason = "...")]` for exceptions, never `#[allow]`
- No `//` comments, only `///` doc comments
- Explicit `return` in every function and closure
- No `?` operator, no `unwrap()`, no `expect()`
- No `println!`/`eprintln!` — use `writeln!(io::stderr(), ...)`
- No `as` casts — use `From`/`TryFrom`
- No absolute paths — import with `use` first
- All items alphabetically ordered
- Doc comment on every item including private
- Use `discard(expr)` to ignore Results (define `fn discard<T>(_: T) {}`)
