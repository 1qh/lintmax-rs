//! `cargo lintmax` — maximum strictness Rust pipeline in one command.

extern crate alloc;

pub mod analyze;
pub mod comment;
pub mod dprint;
pub mod staleness;
pub mod state;

use alloc::collections::BTreeSet;
use std::env;
use std::fs;
use std::io;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;

use clap::Parser;
use clap::Subcommand;
use serde_json::Value;

/// Embedded clippy configuration.
const CLIPPY_TOML: &str = include_str!("../configs/clippy.toml");
/// Embedded cargo-deny configuration.
const DENY_TOML: &str = include_str!("../configs/deny.toml");
/// Embedded dprint configuration.
const DPRINT_JSON: &str = include_str!("../configs/dprint.json");
/// Embedded rustfmt configuration.
const RUSTFMT_TOML: &str = include_str!("../configs/rustfmt.toml");
/// Embedded typos configuration.
const TYPOS_TOML: &str = include_str!("../configs/typos.toml");

/// Clippy lints to allow (contradicting pairs and impractical restrictions).
#[rustfmt::skip]
const CLIPPY_ALLOW: &[&str] = &[
    "clippy::blanket_clippy_restriction_lints",
    "clippy::exhaustive_enums",
    "clippy::exhaustive_structs",
    "clippy::needless_return",
    "clippy::pattern_type_mismatch",
    "clippy::pub_with_shorthand",
    "clippy::self_named_module_files",
    "clippy::semicolon_if_nothing_returned",
    "clippy::semicolon_outside_block",
    "clippy::separated_literal_suffix",
    "clippy::single_call_fn",
];

/// Clippy lint groups to deny.
#[rustfmt::skip]
const CLIPPY_DENY: &[&str] = &[
    "clippy::cargo",
    "clippy::nursery",
    "clippy::pedantic",
    "clippy::restriction",
];

/// Config files managed by lintmax.
const MANAGED_CONFIGS: &[(&str, &str)] = &[
    ("clippy.toml", CLIPPY_TOML),
    ("deny.toml", DENY_TOML),
    ("dprint.json", DPRINT_JSON),
    ("rustfmt.toml", RUSTFMT_TOML),
    ("typos.toml", TYPOS_TOML),
];

/// Rustc lints to deny.
#[rustfmt::skip]
const RUSTC_DENY: &[&str] = &[
    "rust_2018_idioms",
    "unused_extern_crates",
    "unused_qualifications",
    "warnings",
];

/// Rustc lints to forbid.
#[rustfmt::skip]
const RUSTC_FORBID: &[&str] = &[
    "absolute_paths_not_starting_with_crate",
    "ambiguous_negative_literals",
    "closure_returning_async_block",
    "deprecated_in_future",
    "deprecated_safe_2024",
    "deref_into_dyn_supertrait",
    "edition_2024_expr_fragment_specifier",
    "elided_lifetimes_in_paths",
    "explicit_outlives_requirements",
    "ffi_unwind_calls",
    "if_let_rescope",
    "impl_trait_overcaptures",
    "impl_trait_redundant_captures",
    "invalid_type_param_default",
    "keyword_idents_2018",
    "keyword_idents_2024",
    "let_underscore_drop",
    "macro_use_extern_crate",
    "meta_variable_misuse",
    "missing_copy_implementations",
    "missing_debug_implementations",
    "missing_docs",
    "missing_unsafe_on_extern",
    "non_ascii_idents",
    "patterns_in_fns_without_body",
    "redundant_imports",
    "redundant_lifetimes",
    "rust_2021_incompatible_closure_captures",
    "rust_2021_incompatible_or_patterns",
    "rust_2021_prefixes_incompatible_syntax",
    "rust_2021_prelude_collisions",
    "rust_2024_guarded_string_incompatible_syntax",
    "rust_2024_incompatible_pat",
    "rust_2024_prelude_collisions",
    "single_use_lifetimes",
    "tail_expr_drop_order",
    "trivial_casts",
    "trivial_numeric_casts",
    "unit_bindings",
    "unnameable_types",
    "unreachable_pub",
    "unsafe_attr_outside_unsafe",
    "unsafe_code",
    "unsafe_op_in_unsafe_fn",
    "unstable_features",
    "unused_crate_dependencies",
    "unused_import_braces",
    "unused_lifetimes",
    "unused_macro_rules",
    "unused_results",
    "variant_size_differences",
];

/// Cargo wrapper for subcommand dispatch.
#[derive(Parser)]
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
enum Cargo {
    /// Maximum strictness Rust pipeline.
    Lintmax(Cli),
}

/// CLI arguments.
#[derive(Parser)]
#[command(version, about = "Maximum strictness Rust pipeline")]
struct Cli {
    /// Subcommand to run.
    #[command(subcommand)]
    command: Option<Sub>,
}

/// Available subcommands.
#[derive(Subcommand)]
enum Sub {
    /// CI verify: read-only full gate, no writes.
    Check,
    /// Auto-fix everything then verify (the default action).
    Fix,
    /// List the active rule set.
    Rules,
    /// Print the version.
    Version,
}

/// Discards a result, satisfying must-use and drop lints.
fn discard<T>(_value: T) {}

/// Removes temporary config files lintmax owns: an exact embedded match, or a
/// dprint.json that is the embedded default with only its plugin versions bumped.
fn clean_configs() {
    for (name, content) in MANAGED_CONFIGS {
        let path = config_path(name);
        let owned = is_lintmax_content(&path, content)
            || (*name == "dprint.json" && is_bumped_dprint(&path, content));
        if owned {
            discard(fs::remove_file(path));
        }
    }
}

/// Strips the version segment from a single dprint plugin URL line.
fn normalize_dprint_line(line: &str) -> String {
    let is_plugin = line
        .trim_start()
        .starts_with("\"https://plugins.dprint.dev/");
    if let (true, Some(start)) = (is_plugin, line.rfind('/')) {
        return line.get(..=start).unwrap_or(line).to_owned();
    }
    return line.to_owned();
}

/// Drops the `-<version>` segment from every dprint plugin URL so a bumped
/// config compares equal to the embedded seed.
fn normalize_dprint(text: &str) -> String {
    return text
        .lines()
        .map(normalize_dprint_line)
        .collect::<Vec<_>>()
        .join("\n");
}

/// Whether the file is the embedded dprint.json with only plugin versions changed.
fn is_bumped_dprint(path: &Path, embedded: &str) -> bool {
    return fs::read_to_string(path)
        .is_ok_and(|content| return normalize_dprint(&content) == normalize_dprint(embedded));
}

/// Runs an external command.
fn cmd(program: &str, args: &[&str]) -> ExitCode {
    return cmd_env(program, args, &[]);
}

/// Runs an external command with environment variables.
fn cmd_env(program: &str, args: &[&str], env_vars: &[(&str, &str)]) -> ExitCode {
    let mut command = Command::new(program);
    discard(command.args(args));
    for &(key, val) in env_vars {
        discard(command.env(key, val));
    }
    return match command.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(_) => ExitCode::FAILURE,
    };
}

/// Runs a command, buffering its output; prints captured stdout+stderr only on
/// failure so a clean run stays silent (token-efficient `ok`-on-success).
fn cmd_quiet(program: &str, args: &[&str]) -> ExitCode {
    let output = Command::new(program).args(args).output();
    return match output {
        Ok(out) if out.status.success() => ExitCode::SUCCESS,
        Ok(out) => {
            discard(io::stdout().write_all(&out.stdout));
            discard(io::stderr().write_all(&out.stderr));
            ExitCode::from(u8::try_from(out.status.code().unwrap_or(1)).unwrap_or(1))
        }
        Err(_) => ExitCode::FAILURE,
    };
}

/// Returns path for a config file name.
fn config_path(name: &str) -> PathBuf {
    return PathBuf::from(name);
}

/// Checks if file content matches expected embedded content.
fn is_lintmax_content(path: &Path, expected: &str) -> bool {
    return fs::read_to_string(path).map_or(true, |content| return content == expected);
}

/// Entry point.
fn main() -> ExitCode {
    let Cargo::Lintmax(cli) = Cargo::parse();

    refresh_toolchain();
    match cli.command {
        None | Some(Sub::Check) => return run_default(),
        Some(Sub::Version) => {
            emit(pkg_version());
            return ExitCode::SUCCESS;
        }
        Some(Sub::Rules) => {
            print_rules();
            return ExitCode::SUCCESS;
        }
        Some(Sub::Fix) => {}
    }
    let result = run_fix();
    if result == ExitCode::SUCCESS {
        emit("ok");
    }
    return result;
}

/// Runs all checks with temporary configs (no green-cache; used by CI paths).
fn run_check_all() -> ExitCode {
    write_configs();
    let result = run_seq(&[
        run_deny,
        run_doc,
        run_fmt_check,
        run_lint,
        run_machete,
        run_no_comments,
        run_shellcheck,
        run_shfmt_check,
        run_test,
        run_typos,
    ]);
    run_advisories();
    clean_configs();
    return result;
}

/// Cargo package version, baked in at compile time.
const fn pkg_version() -> &'static str {
    return env!("CARGO_PKG_VERSION");
}

/// Whether the gate runs under CI (where the green-cache is bypassed so a fresh
/// run always validates).
fn in_ci() -> bool {
    return env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
}

/// Default gate: short-circuits with `ok (cached)` when the working tree is
/// unchanged since the last clean run, otherwise runs the full gate and records
/// the green tree-hash on success.
fn run_default() -> ExitCode {
    let key = (!in_ci())
        .then(|| return state::tree_hash(pkg_version()))
        .flatten();
    if let (Some(hash), Some(cwd)) = (key.as_ref(), state::cwd_key())
        && state::load().last_green_by_cwd.get(&cwd) == Some(hash)
    {
        emit("ok (cached)");
        return ExitCode::SUCCESS;
    }
    let result = run_check_all();
    if result == ExitCode::SUCCESS {
        persist_green(key.as_ref());
        emit("ok");
    }
    return result;
}

/// Records the current tree-hash as the cwd's last green run.
fn persist_green(hash: Option<&String>) {
    if let (Some(digest), Some(cwd)) = (hash, state::cwd_key()) {
        let mut st = state::load();
        discard(st.last_green_by_cwd.insert(cwd, digest.clone()));
        st.save();
    }
}

/// Writes a line to stdout and flushes.
fn emit(line: &str) {
    let mut stdout = io::stdout();
    discard(writeln!(stdout, "{line}"));
    discard(stdout.flush());
}

/// Emits one advisory block to stderr when its body is non-empty.
fn advisory(prefix: &str, body: &str) {
    if !body.is_empty() {
        discard(write!(io::stderr(), "advisory: {prefix}{body}"));
    }
}

/// Runs the non-failing in-house advisories.
///
/// Covers the dependency staleness scan plus the dupconst, gibberish-identifier,
/// and unguarded-float-division analyzers. All print to stderr and never change
/// the exit code (advisory phases).
fn run_advisories() {
    let stale = staleness::scan(Path::new("."));
    advisory(
        &format!(
            "{} dep(s) behind latest (bump toward active-maintenance window):\n",
            stale.len()
        ),
        &staleness::format(&stale),
    );
    let dups = analyze::dupconst();
    advisory(
        &format!(
            "{} duplicate-value const group(s) (collapse to one):\n",
            dups.len()
        ),
        &analyze::format_dupconst(&dups),
    );
    advisory("", &analyze::format_gibberish(&analyze::gibberish()));
    let fdiv = analyze::floatdiv();
    advisory(
        &format!(
            "{} unguarded float-division site(s) (NaN/Inf risk on empty input):\n",
            fdiv.len()
        ),
        &analyze::format_floatdiv(&fdiv),
    );
}

/// Builds clippy lint flags.
fn build_lint_args() -> Vec<String> {
    let mut args = Vec::new();
    for lint in RUSTC_FORBID {
        args.push("-F".into());
        args.push((*lint).into());
    }
    for lint in RUSTC_DENY {
        args.push("-D".into());
        args.push((*lint).into());
    }
    for lint in CLIPPY_DENY {
        args.push("-D".into());
        args.push((*lint).into());
    }
    for lint in CLIPPY_ALLOW {
        args.push("-A".into());
        args.push((*lint).into());
    }
    return args;
}

/// Runs clippy auto-fix with all lint flags.
fn run_clippy_fix() -> ExitCode {
    let mut args: Vec<String> = vec![
        "clippy".into(),
        "--all-targets".into(),
        "--all-features".into(),
        "--fix".into(),
        "--allow-dirty".into(),
        "--quiet".into(),
        "--".into(),
    ];
    args.extend(build_lint_args());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("cargo", &refs);
}

/// Duplicate crate names when every cargo-deny error is a duplicate; None if any other error appears.
fn duplicate_only_failures(stderr: &str) -> Option<Vec<String>> {
    let mut dups = Vec::new();
    for line in stderr.lines() {
        if !line.contains("error[") {
            continue;
        }
        if !line.contains("error[duplicate]") {
            return None;
        }
        let name = line
            .split("for crate '")
            .nth(1)
            .and_then(|rest| return rest.split('\'').next());
        if let Some(found) = name {
            dups.push(found.to_owned());
        }
    }
    return Some(dups);
}

/// Every dependency name declared by a workspace package in cargo metadata.
fn collect_dep_names(meta: &Value) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let packages = meta.get("packages").and_then(Value::as_array);
    let arrays = packages
        .into_iter()
        .flatten()
        .filter_map(|pkg| return pkg.get("dependencies"))
        .filter_map(Value::as_array);
    for dep in arrays.flatten() {
        if let Some(name) = dep.get("name").and_then(Value::as_str) {
            discard(set.insert(name.to_owned()));
        }
    }
    return set;
}

/// Names every first-party (workspace) crate depends on directly, via cargo metadata.
fn first_party_direct_deps() -> BTreeSet<String> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output();
    let Ok(out) = output else {
        return BTreeSet::new();
    };
    return serde_json::from_slice::<Value>(&out.stdout)
        .map(|meta| return collect_dep_names(&meta))
        .unwrap_or_default();
}

/// Duplicates safe to suppress: every cargo-deny error is a duplicate no first-party crate causes.
fn suppressible_duplicates(stderr: &str) -> Option<Vec<String>> {
    let dups = match duplicate_only_failures(stderr) {
        Some(found) => found,
        None => return None,
    };
    if dups.is_empty() {
        return None;
    }
    let first_party = first_party_direct_deps();
    if dups
        .iter()
        .any(|name| return first_party.contains(name.as_str()))
    {
        return None;
    }
    return Some(dups);
}

/// Runs cargo-deny; suppresses only upstream-transitive duplicates the project cannot fix.
fn run_deny() -> ExitCode {
    let output = match Command::new("cargo")
        .args(["deny", "-L", "error", "check"])
        .output()
    {
        Ok(out) => out,
        Err(_) => return ExitCode::FAILURE,
    };
    if output.status.success() {
        return ExitCode::SUCCESS;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if let Some(dups) = suppressible_duplicates(&stderr) {
        discard(writeln!(
            io::stderr(),
            "deny: {} upstream-transitive duplicate(s) suppressed (unfixable here): {}",
            dups.len(),
            dups.join(", ")
        ));
        return ExitCode::SUCCESS;
    }
    discard(io::stdout().write_all(&output.stdout));
    discard(io::stderr().write_all(&output.stderr));
    return ExitCode::from(u8::try_from(output.status.code().unwrap_or(1)).unwrap_or(1));
}

/// Builds docs with warnings denied.
fn run_doc() -> ExitCode {
    return cmd_env(
        "cargo",
        &["doc", "--no-deps", "--all-features", "--quiet"],
        &[("RUSTDOCFLAGS", "-D warnings")],
    );
}

/// Auto-fixes clippy, comments, typos, and formatting.
fn run_fix() -> ExitCode {
    write_configs();
    let result = run_seq(&[
        run_clippy_fix,
        run_remove_comments,
        run_typos_fix,
        run_shfmt_fix,
        run_fmt_all,
    ]);
    clean_configs();
    return result;
}

/// Formats rust and all other files.
fn run_fmt_all() -> ExitCode {
    discard(cmd("cargo", &["fmt", "--all"]));
    discard(cmd("dprint", &["fmt"]));
    return ExitCode::SUCCESS;
}

/// Checks formatting of rust and all other files.
fn run_fmt_check() -> ExitCode {
    let result_rust = cmd("cargo", &["fmt", "--all", "--", "--check"]);
    let result_dprint = cmd("dprint", &["check"]);
    return worst(result_rust, result_dprint);
}

/// Refreshes the toolchain to latest on a cadence: every run under CI, otherwise
/// at most once per refresh window so the fast local loop stays cheap.
fn refresh_toolchain() {
    if in_ci() {
        do_refresh();
        return;
    }
    if state::refresh_due() {
        do_refresh();
        state::mark_refreshed();
    }
}

/// Bumps cargo deps and dprint plugins to latest.
fn do_refresh() {
    discard(cmd_quiet("cargo", &["update"]));
    discard(cmd_quiet("dprint", &["config", "update"]));
}

/// Runs clippy with all lint flags.
fn run_lint() -> ExitCode {
    let mut args: Vec<String> = vec![
        "clippy".into(),
        "--all-targets".into(),
        "--all-features".into(),
        "--quiet".into(),
        "--".into(),
    ];
    args.extend(build_lint_args());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("cargo", &refs);
}

/// Runs cargo-machete unused dependency check.
fn run_machete() -> ExitCode {
    return cmd_quiet("cargo", &["machete"]);
}

/// Collects the stdout lines of an rg invocation as deduplicated paths,
/// appending any not already present.
fn collect_rg(args: &[&str], paths: &mut Vec<PathBuf>) {
    let Ok(out) = Command::new("rg").args(args).output() else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let path = PathBuf::from(line);
        if !line.is_empty() && !paths.contains(&path) {
            paths.push(path);
        }
    }
}

/// Hand-written shell scripts in the tree (by `.sh` extension or a `#!...sh`
/// shebang), excluding generated output — the shell gate's scan scope.
fn shell_files() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = Vec::new();
    collect_rg(
        &[
            "--hidden",
            "--glob",
            "!.git/**",
            "--glob",
            "!target/**",
            "--files-with-matches",
            "-U",
            r"\A#!.*\bsh\b",
        ],
        &mut paths,
    );
    collect_rg(
        &[
            "--hidden",
            "--glob",
            "!.git/**",
            "--glob",
            "!target/**",
            "--files",
            "--glob",
            "*.sh",
        ],
        &mut paths,
    );
    return paths;
}

/// Lints every shell script with shellcheck at max severity (all optional
/// checks on). A clean run is silent; findings surface on failure.
fn run_shellcheck() -> ExitCode {
    let files = shell_files();
    if files.is_empty() {
        return ExitCode::SUCCESS;
    }
    let mut args: Vec<String> = vec!["--severity=style".into(), "--enable=all".into()];
    for path in &files {
        args.push(path.display().to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("shellcheck", &refs);
}

/// Verifies every shell script is shfmt-formatted (shell auto-detected from the
/// shebang, 2-space indent).
fn run_shfmt_check() -> ExitCode {
    return run_shfmt("-d");
}

/// Formats every shell script in place with shfmt.
fn run_shfmt_fix() -> ExitCode {
    return run_shfmt("-w");
}

/// Runs shfmt over every shell script with the given mode flag.
fn run_shfmt(mode: &str) -> ExitCode {
    let files = shell_files();
    if files.is_empty() {
        return ExitCode::SUCCESS;
    }
    let mut args: Vec<String> = vec!["-i=2".into(), mode.into()];
    for path in &files {
        args.push(path.display().to_string());
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("shfmt", &refs);
}

/// Source files scanned for comments (any hand-written `.rs` under `src/`).
fn source_files() -> Vec<PathBuf> {
    let output = Command::new("rg")
        .args(["--files", "-t", "rust", "src/"])
        .output();
    return match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|line| return !line.is_empty())
            .map(PathBuf::from)
            .collect(),
        Err(_) => Vec::new(),
    };
}

/// Reports any non-survivor `//` comment lines in one file, returning if found.
fn report_comments(path: &Path, content: &str) -> bool {
    let mut found = false;
    for (num, line) in content.lines().enumerate() {
        if comment::strip_line(line).1 {
            found = true;
            discard(writeln!(
                io::stderr(),
                "{}:{}: // comment (only /// and //! doc comments allowed)",
                path.display(),
                num.saturating_add(1)
            ));
        }
    }
    return found;
}

/// Checks that no non-survivor `//` comments exist in rust source.
fn run_no_comments() -> ExitCode {
    let mut found = false;
    for path in source_files() {
        if let Ok(content) = fs::read_to_string(&path) {
            found |= report_comments(&path, &content);
        }
    }
    if found {
        return ExitCode::FAILURE;
    }
    return ExitCode::SUCCESS;
}

/// Strips comments from a file's content, returning the new text if it changed.
fn strip_content(content: &str) -> Option<String> {
    let mut changed = false;
    let mut out_lines: Vec<String> = Vec::new();
    for line in content.lines() {
        let (stripped, removed) = comment::strip_line(line);
        changed |= removed;
        if removed && stripped.is_empty() {
            continue;
        }
        out_lines.push(stripped);
    }
    if !changed {
        return None;
    }
    let mut joined = out_lines.join("\n");
    if content.ends_with('\n') {
        joined.push('\n');
    }
    return Some(joined);
}

/// Removes non-survivor `//` comments from rust source files.
fn run_remove_comments() -> ExitCode {
    for path in source_files() {
        if let Ok(content) = fs::read_to_string(&path)
            && let Some(joined) = strip_content(&content)
        {
            discard(fs::write(&path, joined));
        }
    }
    return ExitCode::SUCCESS;
}

/// Runs steps sequentially, stopping on first failure.
fn run_seq(steps: &[fn() -> ExitCode]) -> ExitCode {
    for step in steps {
        let code = step();
        if code != ExitCode::SUCCESS {
            return code;
        }
    }
    return ExitCode::SUCCESS;
}

/// Runs tests with nextest and doc tests.
fn run_test() -> ExitCode {
    let result = cmd_quiet(
        "cargo",
        &[
            "nextest",
            "run",
            "--all-features",
            "--no-tests=pass",
            "--status-level=none",
            "--final-status-level=fail",
        ],
    );
    discard(
        Command::new("cargo")
            .args(["test", "--doc", "--quiet"])
            .stderr(Stdio::null())
            .status(),
    );
    return result;
}

/// Checks for typos in source.
fn run_typos() -> ExitCode {
    return cmd("typos", &[]);
}

/// Auto-fixes typos in source.
fn run_typos_fix() -> ExitCode {
    return cmd("typos", &["-w"]);
}

/// Prints the active rule set: every clippy group denied, the rustc forbid/deny
/// sets, and the in-house advisory analyzers.
fn print_rules() {
    let mut out = io::stdout();
    discard(writeln!(
        out,
        "clippy groups (deny): {}",
        CLIPPY_DENY.join(", ")
    ));
    discard(writeln!(
        out,
        "clippy allow (contradicting pairs / impractical): {}",
        CLIPPY_ALLOW.join(", ")
    ));
    discard(writeln!(out, "rustc forbid: {}", RUSTC_FORBID.join(", ")));
    discard(writeln!(out, "rustc deny: {}", RUSTC_DENY.join(", ")));
    discard(writeln!(
        out,
        "in-house analyzers: dupconst, gibberish, floatdiv"
    ));
    discard(writeln!(
        out,
        "gates: fmt(rustfmt+dprint), shell(shellcheck+shfmt), typos, no-comments, clippy, doc, test, cargo-deny, cargo-machete"
    ));
    discard(out.flush());
}

/// Returns the worse of two exit codes.
fn worst(first: ExitCode, second: ExitCode) -> ExitCode {
    if first != ExitCode::SUCCESS {
        return first;
    }
    return second;
}

/// Writes a config file if it does not exist or matches embedded content.
fn write_config(name: &str, content: &str) {
    let path = config_path(name);
    if path.exists() && !is_lintmax_content(&path, content) {
        return;
    }
    discard(fs::write(&path, content));
}

/// Writes all temporary config files, then bumps dprint plugins to latest so
/// the embedded version pins are only a bootstrap seed, never a stale lock.
fn write_configs() {
    for (name, content) in MANAGED_CONFIGS {
        write_config(name, content);
    }
    bump_dprint_plugins();
}

/// Rewrites the written dprint config's plugin URLs to latest so the embedded
/// version pins are only a bootstrap seed, never a stale lock.
fn bump_dprint_plugins() {
    let path = config_path("dprint.json");
    if let Ok(content) = fs::read_to_string(&path)
        && let Some(bumped) = dprint::bump(&content)
    {
        discard(fs::write(&path, bumped));
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_dprint;

    /// # Panics
    /// On assertion failure.
    #[test]
    fn normalize_strips_plugin_version() {
        let pinned = "\"https://plugins.dprint.dev/toml-0.7.0.wasm\",";
        let other = "\"https://plugins.dprint.dev/toml-0.9.9.wasm\",";
        assert_eq!(normalize_dprint(pinned), normalize_dprint(other));
    }
}
