//! `cargo lintmax` — maximum strictness Rust pipeline in one command.

extern crate alloc;

pub mod analyze;
pub mod comment;
pub mod dprint;
pub mod reorder;
pub mod staleness;
pub mod state;

use alloc::collections::BTreeSet;
use std::{
    env, fs, io,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};
use serde_json::Value;

/// Embedded clippy configuration.
const CLIPPY_TOML: &str = include_str!("../configs/clippy.toml");
/// Embedded cargo-deny configuration.
const DENY_TOML: &str = include_str!("../configs/deny.toml");

/// File a project uses to declare advisories and duplicates it cannot fix.
const EXCEPTIONS_FILE: &str = "lintmax-exceptions.toml";
/// Embedded dprint configuration.
const DPRINT_JSON: &str = include_str!("../configs/dprint.json");
/// Embedded rustfmt configuration.
const RUSTFMT_TOML: &str = include_str!("../configs/rustfmt.toml");
/// Embedded typos configuration.
const TYPOS_TOML: &str = include_str!("../configs/typos.toml");
/// Marker delimiting the computed vendored-ignore block appended to rustfmt.toml.
const RUSTFMT_IGNORE_MARKER: &str = "\n# lintmax: vendored crates excluded from formatting\n";

/// Clippy lints to allow (contradicting pairs, impractical restrictions, and
/// duplicates of a sibling stage that gates the same class more precisely).
#[rustfmt::skip]
const CLIPPY_ALLOW: &[&str] = &[
    "clippy::blanket_clippy_restriction_lints",
    "clippy::float_arithmetic",
    "clippy::multiple_crate_versions",
    "clippy::needless_return",
    "clippy::pub_with_shorthand",
    "clippy::redundant_pub_crate",
    "clippy::ref_patterns",
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
    "clippy::all",
    "deprecated_safe",
    "future_incompatible",
    "keyword_idents",
    "let_underscore",
    "nonstandard_style",
    "refining_impl_trait",
    "rust_2018_compatibility",
    "rust_2018_idioms",
    "rust_2021_compatibility",
    "rust_2024_compatibility",
    "unknown_or_malformed_diagnostic_attributes",
    "unused",
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
    "linker_info",
    "linker_messages",
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

/// Exceptions a project declares for advisories it cannot act on.
///
/// A dependency reached only through a third party pins what a project may use,
/// so an advisory or duplicate it introduces is not something the project can
/// fix by changing its own manifest. Naming each one keeps the check ON: the
/// gate still fails the moment a DIFFERENT advisory or duplicate appears.
#[derive(Debug, Default, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Exceptions {
    /// Advisory identifiers to ignore, each with a reason in the project's ADR.
    #[serde(default)]
    advisories: Vec<String>,
    /// `crate@version` entries whose duplication is forced by a dependency.
    #[serde(default)]
    duplicates: Vec<String>,
    /// Paths a GENERATOR owns, which the formatter must leave exactly as written.
    ///
    /// A generated file belongs to the tool that emits it, so formatting it puts
    /// the gate and that generator in a loop where each undoes the other — and
    /// the generator then reports its own output as out of date forever, which
    /// reads as a stale config rather than as two tools disagreeing. This is
    /// DATA about one project rather than a rule, which is why it is declared
    /// here and never by relaxing a check.
    #[serde(default)]
    generated: Vec<String>,
}

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

/// Cargo package version, baked in at compile time.
const fn pkg_version() -> &'static str {
    return env!("CARGO_PKG_VERSION");
}

/// Discards a result, satisfying must-use and drop lints.
fn discard<T>(_value: T) {}

/// Removes temporary config files lintmax owns: an exact embedded match, or a
/// dprint.json that is the embedded default with only its plugin versions bumped.
fn clean_configs() {
    for &(name, content) in MANAGED_CONFIGS {
        let path = config_path(name);
        let owned = is_lintmax_content(&path, content)
            || (name == "deny.toml" && is_lintmax_content(&path, &deny_with_exceptions(content)))
            || (name == "dprint.json" && is_bumped_dprint(&path, content))
            || (name == "rustfmt.toml" && is_lintmax_rustfmt(&path));
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
        },
        Err(_) => ExitCode::FAILURE,
    };
}

/// Resolves the nightly rustfmt binary path used for strict (nightly-only) options.
fn nightly_rustfmt() -> Option<String> {
    let Ok(out) = Command::new("rustup")
        .args(["which", "--toolchain", "nightly", "rustfmt"])
        .output()
    else {
        return None;
    };
    if !out.status.success() {
        return None;
    }
    let Ok(text) = String::from_utf8(out.stdout) else {
        return None;
    };
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    return Some(trimmed.to_owned());
}

/// Ensures the active toolchain carries one rustup component.
///
/// A `--profile minimal` toolchain (the common CI install) omits `rustfmt` and
/// `clippy`, so their `cargo` subcommand wrappers (`cargo-fmt` / `cargo-clippy`)
/// are absent and the stage dies with "not installed for the toolchain". This adds
/// the component when its probe fails. Idempotent.
fn ensure_active_component(probe: &[&str], component: &str) {
    let present = Command::new("cargo")
        .args(probe)
        .output()
        .is_ok_and(|out| return out.status.success());
    if present {
        return;
    }
    discard(cmd_quiet("rustup", &["component", "add", component]));
}

/// Ensures every active-toolchain component a gate stage shells out to is present.
///
/// `rustfmt` (the fmt stage) and `clippy` (the lint stage). The `RUSTFMT` env still
/// forces the strict nightly rustfmt binary — these only provide the wrappers.
fn ensure_active_components() {
    ensure_active_component(&["fmt", "--version"], "rustfmt");
    ensure_active_component(&["clippy", "--version"], "clippy");
}

/// Returns the nightly rustfmt path, installing the toolchain + component if absent.
fn require_nightly_rustfmt() -> Option<String> {
    if let Some(path) = nightly_rustfmt() {
        return Some(path);
    }
    discard(cmd_quiet("rustup", &[
        "toolchain",
        "install",
        "nightly",
        "--component",
        "rustfmt",
        "--profile",
        "minimal",
    ]));
    return nightly_rustfmt();
}

/// Returns path for a config file name.
fn config_path(name: &str) -> PathBuf {
    return PathBuf::from(name);
}

/// Checks if file content matches expected embedded content.
fn is_lintmax_content(path: &Path, expected: &str) -> bool {
    return fs::read_to_string(path).map_or(true, |content| return content == expected);
}

/// The rustfmt config written for the project; the base config verbatim.
///
/// Vendored crates are excluded by formatting workspace members explicitly
/// (`cargo fmt -p`), not by a rustfmt `ignore` block — an appended block also
/// makes the written `rustfmt.toml` itself dprint-dirty under a strict config.
fn rustfmt_with_ignores() -> String {
    return RUSTFMT_TOML.to_owned();
}

/// Whether the file is the embedded rustfmt.toml, optionally carrying the
/// computed vendored-ignore block appended after the marker.
fn is_lintmax_rustfmt(path: &Path) -> bool {
    let Ok(content) = fs::read_to_string(path) else {
        return true;
    };
    let head = content
        .split(RUSTFMT_IGNORE_MARKER)
        .next()
        .unwrap_or(&content);
    return head == RUSTFMT_TOML;
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
        },
        Some(Sub::Rules) => {
            print_rules();
            return ExitCode::SUCCESS;
        },
        Some(Sub::Fix) => {},
    }
    let result = run_fix();
    if result == ExitCode::SUCCESS {
        emit("ok");
    }
    return result;
}

/// Runs all checks with temporary configs (no green-cache; used by CI paths).
fn run_check_all() -> ExitCode {
    ensure_active_components();
    write_configs();
    let result = run_seq(&[
        run_deny,
        run_doc,
        run_feature_matrix,
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
        "--workspace".into(),
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

/// Duplicate crate names when every cargo-deny error is a duplicate; None if any other error
/// appears.
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

/// Package ids that are workspace (first-party) members.
fn workspace_member_ids(meta: &Value) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let members = meta.get("workspace_members").and_then(Value::as_array);
    for id in members.into_iter().flatten() {
        if let Some(text) = id.as_str() {
            discard(set.insert(text.to_owned()));
        }
    }
    return set;
}

/// True when a package is local (path/null source) and not a workspace member:
/// vendored external code we consume but do not author.
fn is_vendored_package(pkg: &Value, members: &BTreeSet<String>) -> bool {
    let local = pkg.get("source").is_none_or(Value::is_null);
    let member = pkg
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| return members.contains(id));
    return local && !member;
}

/// The `dir/**` exclude glob for a package's directory, relative to cwd.
fn package_dir_glob(pkg: &Value, cwd: &Path) -> Option<String> {
    let manifest = match pkg.get("manifest_path").and_then(Value::as_str) {
        Some(text) => text,
        None => return None,
    };
    let dir = match Path::new(manifest).parent() {
        Some(parent) => parent,
        None => return None,
    };
    let rel = dir.strip_prefix(cwd).unwrap_or(dir);
    return Some(format!("{}/**", rel.display()));
}

/// Exclude globs for vendored external packages (local non-workspace crates).
///
/// Like registry dependencies, vendored crates are compiled but not linted;
/// detected via cargo metadata, never by directory name, so no first-party
/// source can be excluded by accident.
fn vendored_excludes() -> Vec<String> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1"])
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    let Ok(meta) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Vec::new();
    };
    let members = workspace_member_ids(&meta);
    let cwd = env::current_dir().unwrap_or_default();
    let packages = meta.get("packages").and_then(Value::as_array);
    return packages
        .into_iter()
        .flatten()
        .filter(|pkg| return is_vendored_package(pkg, &members))
        .filter_map(|pkg| return package_dir_glob(pkg, &cwd))
        .collect();
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
        &[
            "doc",
            "--workspace",
            "--no-deps",
            "--all-features",
            "--quiet",
        ],
        &[(
            "RUSTDOCFLAGS",
            "-D warnings -D rustdoc::missing_crate_level_docs -D rustdoc::private_doc_tests -D \
             rustdoc::unescaped_backticks",
        )],
    );
}

/// Auto-fixes clippy, comments, typos, and formatting.
fn run_fix() -> ExitCode {
    ensure_active_components();
    write_configs();
    let fixed = run_seq(&[
        run_clippy_fix,
        run_reorder_items,
        run_remove_comments,
        run_typos_fix,
        run_shfmt_fix,
        run_fmt_all,
    ]);
    clean_configs();
    if fixed != ExitCode::SUCCESS {
        return fixed;
    }
    return run_check_all();
}

/// Sorts every module's items into the groups the ordering lint declares.
///
/// The rewrite refuses itself whenever a file's token multiset would change, so
/// a misparse leaves the file untouched rather than losing a declaration.
fn run_reorder_items() -> ExitCode {
    reorder::sort_tree(Path::new("."));
    return ExitCode::SUCCESS;
}

/// Formats rust and all other files, gating on a formatter that fails (e.g. a
/// line rustfmt cannot fit under `max_width` when `error_on_line_overflow` is on).
fn run_fmt_all() -> ExitCode {
    let Some(rustfmt) = require_nightly_rustfmt() else {
        emit("nightly rustfmt unavailable; required for strict formatting");
        return ExitCode::FAILURE;
    };
    let result_rust = run_fmt_members(&rustfmt, &[]);
    let result_dprint = run_dprint("fmt");
    return worst(result_rust, result_dprint);
}

/// Names of the workspace-member packages, for formatting them explicitly.
fn workspace_member_names() -> Vec<String> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    let Ok(meta) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Vec::new();
    };
    let members = workspace_member_ids(&meta);
    let packages = meta.get("packages").and_then(Value::as_array);
    let mut names = Vec::new();
    for pkg in packages.into_iter().flatten() {
        let is_member = pkg
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| return members.contains(id));
        if is_member && let Some(name) = pkg.get("name").and_then(Value::as_str) {
            names.push(name.to_owned());
        }
    }
    return names;
}

/// Runs cargo fmt over each workspace member explicitly, never `--all`.
///
/// `--all` also walks excluded vendored path-deps, and the rustfmt `ignore` glob
/// that would re-exclude them is honored inconsistently across rustfmt versions.
fn run_fmt_members(rustfmt: &str, extra: &[&str]) -> ExitCode {
    let names = workspace_member_names();
    let mut args: Vec<String> = vec!["fmt".to_owned()];
    if names.is_empty() {
        args.push("--all".to_owned());
    } else {
        for name in &names {
            args.push("-p".to_owned());
            args.push(name.clone());
        }
    }
    if !extra.is_empty() {
        args.push("--".to_owned());
        for entry in extra {
            args.push((*entry).to_owned());
        }
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd_env("cargo", &refs, &[("RUSTFMT", rustfmt)]);
}

/// Checks formatting of rust and all other files.
fn run_fmt_check() -> ExitCode {
    let Some(rustfmt) = require_nightly_rustfmt() else {
        emit("nightly rustfmt unavailable; required for strict formatting");
        return ExitCode::FAILURE;
    };
    let result_rust = run_fmt_members(&rustfmt, &["--check"]);
    let result_dprint = run_dprint("check");
    return worst(result_rust, result_dprint);
}

/// Runs a dprint action, excluding vendored external package directories.
fn run_dprint(action: &str) -> ExitCode {
    let mut args = vec![action.to_owned()];
    let globs = vendored_excludes();
    if !globs.is_empty() {
        args.push("--excludes".to_owned());
        args.extend(globs);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("dprint", &refs);
}

/// Runs typos with the given extra args, excluding vendored external directories.
fn run_typos_excluded(extra: &[&str]) -> ExitCode {
    let mut args: Vec<String> = extra.iter().map(|arg| return (*arg).to_owned()).collect();
    for glob in vendored_excludes() {
        args.push("--exclude".to_owned());
        args.push(glob);
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("typos", &refs);
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

/// Lints every feature combination, not only the all-features one.
///
/// `--all-features` proves one combination compiles and lints; a crate can pass
/// that and break on a subset, because a `cfg`-gated item is only missing when
/// its feature is off. This costs nothing on a crate that declares no features
/// and scales with the feature count on one that does.
fn run_feature_matrix() -> ExitCode {
    let mut args: Vec<String> = vec![
        "hack".into(),
        "clippy".into(),
        "--each-feature".into(),
        "--workspace".into(),
        "--all-targets".into(),
        "--quiet".into(),
        "--".into(),
    ];
    args.extend(build_lint_args());

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("cargo", &refs);
}

/// Runs the cargo-machete unused dependency check.
///
/// Invoked as the binary with an EXPLICIT path. The `cargo machete` form relies
/// on cargo-machete stripping its own name and it instead walks `machete` as a
/// path; the bare binary panics on some versions for want of any argument. A
/// path satisfies both, and neither failure looks like an invocation bug.
fn run_machete() -> ExitCode {
    let excludes = vendored_excludes();
    let ignore_path = config_path(".ignore");
    if excludes.is_empty() || ignore_path.exists() {
        return cmd_quiet("cargo-machete", &["."]);
    }
    let body: String = excludes
        .iter()
        .map(|glob| return glob.strip_suffix("**").unwrap_or(glob).to_owned())
        .collect::<Vec<_>>()
        .join("\n");
    if fs::write(&ignore_path, body).is_err() {
        return cmd_quiet("cargo-machete", &["."]);
    }
    let result = cmd_quiet("cargo-machete", &["."]);
    discard(fs::remove_file(&ignore_path));
    return result;
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
    let mut args: Vec<String> = vec![
        "--severity=style".into(),
        "--enable=all".into(),
        "--external-sources".into(),
    ];
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
    let mut args: Vec<String> = vec![
        "-i=2".into(),
        "-ci".into(),
        "-sr".into(),
        "-bn".into(),
        "-s".into(),
        mode.into(),
    ];
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
    for (index, step) in steps.iter().enumerate() {
        let code = step();
        if code != ExitCode::SUCCESS {
            discard(writeln!(
                io::stderr(),
                "lintmax: gate stage #{index} failed"
            ));
            return code;
        }
    }
    return ExitCode::SUCCESS;
}

/// Runs tests with nextest and doc tests.
fn run_test() -> ExitCode {
    let unit = cmd_quiet("cargo", &[
        "nextest",
        "run",
        "--all-features",
        "--no-tests=pass",
        "--status-level=none",
        "--final-status-level=fail",
    ]);
    return worst(unit, run_doctests());
}

/// Runs doctests, gating on real failures. A crate with no library target has no
/// doctests to run, which is success rather than an error.
fn run_doctests() -> ExitCode {
    let output = Command::new("cargo")
        .args(["test", "--workspace", "--doc", "--all-features", "--quiet"])
        .output();
    let Ok(out) = output else {
        return ExitCode::FAILURE;
    };
    if out.status.success() {
        return ExitCode::SUCCESS;
    }
    if String::from_utf8_lossy(&out.stderr).contains("no library targets found") {
        return ExitCode::SUCCESS;
    }
    discard(io::stdout().write_all(&out.stdout));
    discard(io::stderr().write_all(&out.stderr));
    return ExitCode::from(u8::try_from(out.status.code().unwrap_or(1)).unwrap_or(1));
}

/// Checks for typos in source.
fn run_typos() -> ExitCode {
    return run_typos_excluded(&[]);
}

/// Auto-fixes typos in source.
fn run_typos_fix() -> ExitCode {
    return run_typos_excluded(&["-w"]);
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
        "gates: fmt(rustfmt+dprint), shell(shellcheck+shfmt), typos, no-comments, clippy, doc, \
         test, cargo-deny, cargo-machete"
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
    let (final_content, owned) = if name == "rustfmt.toml" {
        (rustfmt_with_ignores(), is_lintmax_rustfmt(&path))
    } else if name == "deny.toml" {
        let merged = deny_with_exceptions(content);
        let owned = is_lintmax_content(&path, &merged);
        (merged, owned)
    } else {
        (content.to_owned(), is_lintmax_content(&path, content))
    };
    if path.exists() && !owned {
        return;
    }
    discard(fs::write(&path, final_content));
}

/// Folds the project's generated paths into the embedded formatter excludes.
///
/// The excludes are extended rather than replaced, so a project can never drop
/// the ones the gate itself relies on — the only thing it may add is a path a
/// generator owns.
fn dprint_with_generated(content: &str) -> String {
    let generated = project_exceptions().generated;
    if generated.is_empty() {
        return content.to_owned();
    }
    let added = generated
        .iter()
        .map(|path| return format!("    \"{path}\",\n"))
        .collect::<String>();
    return content.replacen(
        "  \"excludes\": [\n",
        &format!("  \"excludes\": [\n{added}"),
        1,
    );
}

/// Reads the project's declared exceptions, when it declares any.
///
/// A file that cannot be parsed is reported and then treated as absent, because
/// silently ignoring it would widen the gate by exactly the entries the project
/// believed it had declared.
fn project_exceptions() -> Exceptions {
    let path = Path::new(EXCEPTIONS_FILE);
    let Ok(text) = fs::read_to_string(path) else {
        return Exceptions::default();
    };
    return match toml::from_str::<Exceptions>(&text) {
        Ok(parsed) => parsed,
        Err(error) => {
            advisory(
                EXCEPTIONS_FILE,
                &format!(" is not readable, ignoring it: {error}\n"),
            );
            Exceptions::default()
        },
    };
}

/// Folds the project's exceptions into the embedded cargo-deny configuration.
///
/// Each list is substituted INTO its owning section rather than appended to the
/// file, because a bare append lands in whichever section happens to be last.
fn deny_with_exceptions(content: &str) -> String {
    return merge_exceptions(content, &project_exceptions());
}

/// Substitutes declared exceptions into the config's own sections.
///
/// Each list goes INTO its owning section rather than onto the end of the file,
/// because a bare append lands in whichever section happens to be last.
fn merge_exceptions(content: &str, declared: &Exceptions) -> String {
    let with_advisories = substitute(content, "ignore = []", &declared.advisories, |body| {
        return format!("ignore = [\n{body}\n]");
    });
    return substitute(
        &with_advisories,
        "multiple-versions = \"deny\"",
        &declared.duplicates,
        |body| return format!("multiple-versions = \"deny\"\nskip = [\n{body}\n]"),
    );
}

/// Replaces `anchor` with `shape` applied to the quoted entries, when any.
fn substitute(
    content: &str,
    anchor: &str,
    entries: &[String],
    shape: impl Fn(&str) -> String,
) -> String {
    if entries.is_empty() {
        return content.to_owned();
    }
    let body = entries
        .iter()
        .map(|entry| return format!("  \"{entry}\","))
        .collect::<Vec<String>>()
        .join("\n");
    return content.replace(anchor, &shape(&body));
}

/// Writes all temporary config files, then bumps dprint plugins to latest so
/// the embedded version pins are only a bootstrap seed, never a stale lock.
fn write_configs() {
    ensure_tools();
    for &(name, content) in MANAGED_CONFIGS {
        if name == "dprint.json" {
            write_config(name, &dprint_with_generated(content));
        } else {
            write_config(name, content);
        }
    }
    bump_dprint_plugins();
}

/// Installs any absent cargo child tool the gate shells out to.
///
/// Uses cargo-binstall (fast prebuilt) or cargo install, so a fresh machine
/// never fails the gate on a missing tool.
fn ensure_tools() {
    for &(bin, krate) in &[
        ("cargo-deny", "cargo-deny"),
        ("cargo-hack", "cargo-hack"),
        ("cargo-machete", "cargo-machete"),
        ("cargo-nextest", "cargo-nextest"),
        ("dprint", "dprint"),
        ("typos", "typos-cli"),
    ] {
        ensure_tool(bin, krate);
    }
}

/// Installs one cargo tool when its binary is absent.
///
/// A FORCED install first REMOVES the existing binary, so forcing one that is
/// merely believed absent destroys a working tool the moment the install fails —
/// the force is therefore reserved for a binary that is genuinely not on disk,
/// where cargo would otherwise skip the reinstall on its own registry record and
/// never restore it.
///
/// A tool still absent after the attempt is reported rather than swallowed: the
/// stage would otherwise fail with an unexplained missing command, which reads
/// as a broken toolchain rather than as an install that did not happen.
fn ensure_tool(bin: &str, krate: &str) {
    if installed(bin) {
        return;
    }
    let force: &[&str] = if on_path(bin) { &[] } else { &["--force"] };
    let mut binstall = vec!["binstall", "--no-confirm"];
    binstall.extend_from_slice(force);
    binstall.push(krate);
    if cmd_quiet("cargo", &binstall) != ExitCode::SUCCESS {
        let mut install = vec!["install", "--locked"];
        install.extend_from_slice(force);
        install.push(krate);
        discard(cmd_quiet("cargo", &install));
    }
    if !installed(bin) {
        discard(writeln!(
            io::stderr(),
            "lintmax: could not install {krate}, so {bin} is unavailable"
        ));
    }
}

/// Whether a child tool's binary answers on the current path.
fn installed(bin: &str) -> bool {
    if let Some(sub) = bin.strip_prefix("cargo-") {
        return Command::new("cargo")
            .args([sub, "--version"])
            .output()
            .is_ok_and(|out| return out.status.success());
    }
    return Command::new(bin)
        .arg("--version")
        .output()
        .is_ok_and(|out| return out.status.success());
}

/// Whether a binary of that name sits on the current path.
///
/// A cargo subcommand is invoked THROUGH cargo, so its own file is what says
/// whether an install would be replacing something rather than creating it.
fn on_path(bin: &str) -> bool {
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    for dir in env::split_paths(&path) {
        if dir.join(bin).is_file() {
            return true;
        }
    }
    return false;
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
mod tests;
