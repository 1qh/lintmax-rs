//! `cargo lintmax` — maximum strictness Rust pipeline in one command.

extern crate alloc;

pub mod analyze;
pub mod comment;
pub mod dprint;
pub mod files;
pub mod reorder;
pub mod staleness;
pub mod state;

use alloc::collections::BTreeSet;
use core::{
    cell::RefCell,
    iter,
    sync::atomic::{AtomicU64, Ordering},
};
use std::{
    env,
    ffi::OsStr,
    fs, io,
    io::Write as _,
    path::{Path, PathBuf},
    process,
    process::{Command, ExitCode, ExitStatus, Output},
    sync::OnceLock,
};

use clap::{Parser, Subcommand};
use serde_json::Value;

thread_local! {
    /// Isolated embedded configs used only while checking.
    static CHECK_CONFIGS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Clippy lints to allow (contradicting pairs, impractical restrictions, and
/// duplicates of a sibling stage that gates the same class more precisely).
#[rustfmt::skip]
const CLIPPY_ALLOW: &[&str] = &[
    "clippy::blanket_clippy_restriction_lints",
    "clippy::cognitive_complexity",
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

/// Embedded clippy configuration.
const CLIPPY_TOML: &str = include_str!("../configs/clippy.toml");

/// Embedded cargo-deny configuration.
const DENY_TOML: &str = include_str!("../configs/deny.toml");

/// Embedded dprint configuration.
const DPRINT_JSON: &str = include_str!("../configs/dprint.json");

/// File a project uses to declare advisories and duplicates it cannot fix.
const EXCEPTIONS_FILE: &str = "lintmax-exceptions.toml";

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
    "raw_borrows_via_references",
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

/// Marker delimiting the computed vendored-ignore block appended to rustfmt.toml.
const RUSTFMT_IGNORE_MARKER: &str = "\n# lintmax: vendored crates excluded from formatting\n";

/// Embedded rustfmt configuration.
const RUSTFMT_TOML: &str = include_str!("../configs/rustfmt.toml");

/// Embedded typos configuration.
const TYPOS_TOML: &str = include_str!("../configs/typos.toml");

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

/// Whether an exception source must exist.
#[derive(Clone, Copy)]
enum ExceptionSource {
    /// The conventional project file may be absent.
    Optional,
    /// An explicitly selected file must be readable.
    Required,
}

/// Exceptions a project declares for advisories it cannot act on.
///
/// A dependency reached only through a third party pins what a project may use,
/// so an advisory or duplicate it introduces is not something the project can
/// fix by changing its own manifest. Naming each one keeps the check ON: the
/// gate still fails the moment a DIFFERENT advisory or duplicate appears.
/// An unknown key is IGNORED rather than denied, because consumers upgrade the
/// gate independently: denying one makes a file that names a key this version
/// does not know fail to parse ENTIRELY, which drops every exception the project
/// had already declared and fails its supply-chain stage on advisories that were
/// exempted for good reasons — a failure that points at the dependency tree
/// rather than at the one added key.
#[derive(Debug, Default, serde::Deserialize)]
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
    /// Project vocabulary that the spelling checker must accept unchanged.
    #[serde(default)]
    words: Vec<String>,
}

/// Whether clippy should check or apply fixes.
#[derive(Clone, Copy)]
enum LintMode {
    /// Verify without changing source files.
    Check,
    /// Apply compiler suggestions.
    Fix,
}

/// Optional prebuilt installation strategy.
#[derive(Clone, Copy)]
enum Prebuilt {
    /// Attempt the available prebuilt installer first.
    Available,
    /// Go straight to cargo install.
    Unavailable,
}

/// One `ShellCheck` diagnostic, with its source location and severity.
#[derive(serde::Deserialize)]
struct ShellFinding {
    /// `ShellCheck` warning number.
    code: u64,
    /// First column of the finding.
    column: u64,
    /// Filename carrying the finding.
    file: String,
    /// Severity reported by `ShellCheck`.
    level: String,
    /// First line of the finding.
    line: u64,
    /// Explanation of the finding.
    message: String,
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

/// Injectable tool probes and child execution.
#[derive(Clone, Copy)]
struct ToolInstaller {
    /// Whether the optional prebuilt installer is available.
    prebuilt: Prebuilt,
    /// Checks whether the requested tool is installed.
    probe: fn(&str) -> bool,
    /// Runs an installer with its original arguments.
    run: fn(&str, &[&str]) -> ExitCode,
}

/// A named gate action whose exit status is preserved.
type Stage<'stage> = (&'stage str, fn() -> ExitCode);

/// Child execution methods that always report spawn errors.
trait ToolExecution {
    /// Captures output and reports why a tool could not start.
    ///
    /// # Errors
    /// Returns the underlying error if the required file or child process cannot be read or
    /// started.
    fn tool_output(&mut self) -> io::Result<Output>;
    /// Inherits streams and reports why a tool could not start.
    ///
    /// # Errors
    /// Returns the underlying error if the required file or child process cannot be read or
    /// started.
    fn tool_status(&mut self) -> io::Result<ExitStatus>;
}

impl ToolExecution for Command {
    /// Captures output while reporting a spawn error.
    ///
    /// # Errors
    /// Returns the underlying error if the required file or child process cannot be read or
    /// started.
    fn tool_output(&mut self) -> io::Result<Output> {
        return self.output().inspect_err(|error| {
            return spawn_failure(self.get_program(), error, &mut io::stderr());
        });
    }

    /// Inherits output while reporting a spawn error.
    ///
    /// # Errors
    /// Returns the underlying error if the required file or child process cannot be read or
    /// started.
    fn tool_status(&mut self) -> io::Result<ExitStatus> {
        return self.status().inspect_err(|error| {
            return spawn_failure(self.get_program(), error, &mut io::stderr());
        });
    }
}

/// Emits one advisory block to stderr when its body is non-empty.
fn advisory(prefix: &str, body: &str) {
    if !body.is_empty() {
        discard(write!(io::stderr(), "advisory: {prefix}{body}"));
    }
    return;
}

/// Builds a binary-only clippy pass with the shared flags and binary lint.
fn binary_lint_args(mode: LintMode) -> Vec<String> {
    let mut arguments: Vec<String> = ["clippy", "--workspace", "--bins", "--all-features"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    if matches!(mode, LintMode::Fix) {
        arguments.extend(["--fix".to_owned(), "--allow-dirty".to_owned()]);
    }
    arguments.extend(["--quiet".to_owned(), "--".to_owned()]);
    arguments.extend(build_lint_args());
    arguments.extend(["-F".to_owned(), "dead_code_pub_in_binary".to_owned()]);
    return arguments;
}

/// Cached availability of the optional prebuilt installer.
fn binstall_available() -> bool {
    /// Cached result of the single optional-installer probe.
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    return *AVAILABLE.get_or_init(|| return installed("cargo-binstall"));
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

/// Rewrites the written dprint config's plugin URLs to latest so the embedded
/// version pins are only a bootstrap seed, never a stale lock.
fn bump_dprint_plugins() {
    let path = config_path("dprint.json");
    if let Ok(content) = fs::read_to_string(&path)
        && let Some(bumped) = dprint::bump(&content)
    {
        discard(fs::write(&path, bumped));
    }
    return;
}

/// Computes the local cache key, bypassing the cache under CI.
///
/// # Errors
/// Returns an error if eligible file enumeration fails.
fn cache_key() -> io::Result<Option<String>> {
    if in_ci() {
        return Ok(None);
    }
    let external = env::var_os("LINTMAX_EXCEPTIONS").map(PathBuf::from);
    let version = match cache_version(external.as_deref()) {
        Ok(identity) => identity,
        Err(error) => return Err(error),
    };
    return state::tree_hash(&version).map(Some);
}

/// Includes external exception contents in the green-cache identity.
///
/// # Errors
/// Returns the underlying error if the required file or child process cannot be read or started.
fn cache_version(external: Option<&Path>) -> io::Result<String> {
    let Some(path) = external else {
        return Ok(pkg_version().to_owned());
    };
    return fs::read_to_string(path).map(|content| {
        return format!("{}:{}:{content}", pkg_version(), path.display());
    });
}

/// Whether this directory already passed with the current content identity.
fn cached_green(key: Option<&String>) -> bool {
    return match (key, state::cwd_key()) {
        (Some(hash), Some(cwd)) => state::load().last_green_by_cwd.get(&cwd) == Some(hash),
        _ => false,
    };
}

/// Writes embedded configs outside the project, without installing or refreshing.
///
/// # Errors
/// Returns the underlying error if the required file or child process cannot be read or started.
fn check_configs() -> io::Result<PathBuf> {
    /// Unique scratch-directory sequence within this process.
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let directory = env::temp_dir().join(format!(
        "lintmax-check-{}-{}",
        process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(error) = fs::create_dir_all(&directory) {
        return Err(error);
    }
    if let Err(error) = write_check_configs(&directory) {
        discard(fs::remove_dir_all(&directory));
        return Err(error);
    }
    return Ok(directory);
}

/// Selects the embedded config with the project's declared exceptions.
fn check_content(name: &str, content: &str) -> String {
    return match name {
        "deny.toml" => deny_with_exceptions(content),
        "dprint.json" => dprint_with_generated(content),
        "typos.toml" => typos_with_exceptions(content),
        _ => content.to_owned(),
    };
}

/// Whether this thread is running a read-only verification.
fn checking() -> bool {
    return CHECK_CONFIGS.with(|directory| return directory.borrow().is_some());
}

/// Removes temporary config files lintmax owns: an exact embedded match, or a
/// dprint.json that is the embedded default with only its plugin versions bumped.
fn clean_configs() {
    for &(name, content) in MANAGED_CONFIGS {
        let path = config_path(name);
        let owned = is_lintmax_content(&path, content)
            || (name == "deny.toml" && is_lintmax_content(&path, &deny_with_exceptions(content)))
            || (name == "dprint.json" && is_bumped_dprint(&path, content))
            || (name == "typos.toml" && is_lintmax_content(&path, &typos_with_exceptions(content)))
            || (name == "rustfmt.toml" && is_lintmax_rustfmt(&path));
        if owned {
            discard(fs::remove_file(path));
        }
    }
    return;
}

/// Runs an external command.
fn cmd(program: &str, args: &[&str]) -> ExitCode {
    return cmd_env(program, args, &[]);
}

/// Runs an external command with environment variables.
fn cmd_env(program: &str, args: &[&str], env_vars: &[(&str, &str)]) -> ExitCode {
    let mut command = tool_command(program);
    command_check_env(&mut command, program);
    command_args(&mut command, program, args);
    for &(key, val) in env_vars {
        discard(command.env(key, val));
    }
    return match command.tool_status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1)),
        Err(_) => ExitCode::FAILURE,
    };
}

/// Runs a command, buffering its output; prints captured stdout+stderr only on
/// failure so a clean run stays silent (token-efficient `ok`-on-success).
fn cmd_quiet(program: &str, args: &[&str]) -> ExitCode {
    let mut command = tool_command(program);
    command_args(&mut command, program, args);
    let output = command.tool_output();
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

/// Preserves child arguments while requiring locked resolution during checks.
fn command_args(command: &mut Command, program: &str, arguments: &[&str]) {
    let locked = checking()
        && program == "cargo"
        && arguments.first().is_some_and(|subcommand| {
            return matches!(*subcommand, "clippy" | "doc" | "hack" | "nextest" | "test");
        });
    let separator = arguments
        .iter()
        .position(|argument| return *argument == "--");
    let position = separator.unwrap_or(arguments.len());
    discard(command.args(arguments.get(..position).unwrap_or_default()));
    if locked {
        discard(command.arg("--locked"));
    }
    discard(command.args(arguments.get(position..).unwrap_or_default()));
    return;
}

/// Directs checking cargo children to the isolated clippy configuration.
fn command_check_env(command: &mut Command, program: &str) {
    if checking() && program == "cargo" {
        let config = config_path("clippy.toml");
        if let Some(parent) = config.parent() {
            discard(command.env("CLIPPY_CONF_DIR", parent));
        }
    }
    return;
}

/// Returns path for a config file name.
fn config_path(name: &str) -> PathBuf {
    let local = PathBuf::from(name);
    if local.exists() {
        return local;
    }
    return CHECK_CONFIGS.with(|directory| {
        return directory
            .borrow()
            .as_ref()
            .map_or(local, |root| return root.join(name));
    });
}

/// Folds the project's exceptions into the embedded cargo-deny configuration.
///
/// Each list is substituted INTO its owning section rather than appended to the
/// file, because a bare append lands in whichever section happens to be last.
fn deny_with_exceptions(content: &str) -> String {
    return merge_exceptions(content, &project_exceptions());
}

/// Discards a result, satisfying must-use and drop lints.
fn discard<T>(_value: T) {
    return;
}

/// Dispatches commands without touching the project for informational commands.
fn dispatch(command: Option<&Sub>, check: fn() -> ExitCode, fix: fn() -> ExitCode) -> ExitCode {
    match command {
        Some(&Sub::Check) => return check(),
        Some(&Sub::Version) => {
            emit(pkg_version());
            return ExitCode::SUCCESS;
        },
        Some(&Sub::Rules) => {
            print_rules();
            return ExitCode::SUCCESS;
        },
        None | Some(&Sub::Fix) => return finish_fix(fix),
    }
}

/// Bumps cargo deps and dprint plugins to latest.
fn do_refresh() {
    discard(cmd_quiet("cargo", &["update"]));
    discard(cmd_quiet("dprint", &["config", "update"]));
    return;
}

/// Builds a formatter command while preserving its explicit file arguments.
fn dprint_command(action: &str, patterns: &[String]) -> Command {
    let mut command = tool_command("dprint");
    discard(command.arg(action).args(patterns));
    if checking() {
        discard(command.arg("--config").arg(config_path("dprint.json")));
    }
    return command;
}

/// Converts eligible filenames into literal dprint glob arguments.
///
/// # Errors
/// Returns an error if a filename cannot be represented as UTF-8.
fn dprint_patterns(paths: &[PathBuf]) -> io::Result<Vec<String>> {
    return paths
        .iter()
        .map(|path| {
            return path.to_str().map(literal_glob).ok_or_else(|| {
                return io::Error::new(
                    io::ErrorKind::InvalidData,
                    "dprint cannot safely select a non-UTF-8 filename",
                );
            });
        })
        .collect();
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

/// Writes a line to stdout and flushes.
fn emit(line: &str) {
    let mut stdout = io::stdout();
    discard(writeln!(stdout, "{line}"));
    discard(stdout.flush());
    return;
}

/// Ensures the active toolchain carries one rustup component.
///
/// A `--profile minimal` toolchain (the common CI install) omits `rustfmt` and
/// `clippy`, so their `cargo` subcommand wrappers (`cargo-fmt` / `cargo-clippy`)
/// are absent and the stage dies with "not installed for the toolchain". This adds
/// the component when its probe fails. Idempotent.
fn ensure_active_component(probe: &[&str], component: &str) {
    let present = tool_command("cargo")
        .args(probe)
        .tool_output()
        .is_ok_and(|out| return out.status.success());
    if present {
        return;
    }
    discard(cmd_quiet("rustup", &["component", "add", component]));
    return;
}

/// Ensures every active-toolchain component a gate stage shells out to is present.
///
/// `rustfmt` (the fmt stage) and `clippy` (the lint stage). The `RUSTFMT` env still
/// forces the strict nightly rustfmt binary — these only provide the wrappers.
fn ensure_active_components() {
    ensure_active_component(&["fmt", "--version"], "rustfmt");
    ensure_active_component(&["clippy", "--version"], "clippy");
    return;
}

/// Requests nightly formatting support before the gate starts.
fn ensure_nightly_rustfmt() {
    discard(require_nightly_rustfmt());
    return;
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
    let installer = ToolInstaller {
        prebuilt: if binstall_available() {
            Prebuilt::Available
        } else {
            Prebuilt::Unavailable
        },
        probe: installed,
        run: cmd_quiet,
    };
    ensure_tool_with(bin, krate, &installer);
    return;
}

/// Installs a tool through a probed installer, with injectable process operations.
fn ensure_tool_with(bin: &str, krate: &str, installer: &ToolInstaller) {
    if (installer.probe)(bin) {
        return;
    }
    install_missing_tool(bin, krate, installer);
    if !(installer.probe)(bin) {
        discard(writeln!(
            io::stderr(),
            "lintmax: could not install {krate}, so {bin} is unavailable"
        ));
    }
    return;
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
    return;
}

/// Reports why file enumeration failed and fails the current gate stage.
fn enumeration_failure(error: &io::Error) -> ExitCode {
    discard(writeln!(
        io::stderr(),
        "lintmax: file enumeration failed: {error}"
    ));
    return ExitCode::FAILURE;
}

/// Reports an exception file error and fails before any gate work.
fn exception_failure(error: &io::Error) -> ExitCode {
    discard(writeln!(io::stderr(), "lintmax: {error}"));
    return ExitCode::FAILURE;
}

/// Adds the selected exception path while preserving the IO error kind.
fn exception_path_error(path: &Path, error: &io::Error) -> io::Error {
    return io::Error::new(error.kind(), format!("{}: {error}", path.display()));
}

/// Checks that a candidate is an executable file, rather than a PATH decoy.
fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        return fs::metadata(path)
            .is_ok_and(|metadata| return metadata.permissions().mode() & 0o111 != 0);
    }
    #[cfg(not(unix))]
    {
        return true;
    }
}

/// Removes vendored and generated paths before any explicit-file stage.
fn filter_project_files(
    root: &Path,
    paths: Vec<PathBuf>,
    excludes: &[PathBuf],
    generated: &[String],
) -> Vec<PathBuf> {
    return paths
        .into_iter()
        .filter(|path| {
            let owned = path.strip_prefix(root).ok().and_then(Path::to_str);
            return !excludes
                .iter()
                .any(|exclude| return path.starts_with(exclude))
                && !owned.is_some_and(|relative| {
                    return generated
                        .iter()
                        .any(|pattern| return generated_glob_matches(pattern, relative));
                });
        })
        .collect();
}

/// Prints success after the requested fix completes.
fn finish_fix(fix: fn() -> ExitCode) -> ExitCode {
    let result = fix();
    if result == ExitCode::SUCCESS {
        emit("ok");
    }
    return result;
}

/// Names every first-party (workspace) crate depends on directly, via cargo metadata.
///
/// # Errors
/// Returns the underlying error if the required file or child process cannot be read or started.
fn first_party_direct_deps() -> io::Result<BTreeSet<String>> {
    let mut command = tool_command("cargo");
    discard(command.args(["metadata", "--no-deps", "--format-version", "1"]));
    if checking() {
        discard(command.arg("--locked"));
    }
    let output = command.tool_output();
    let out = match output {
        Ok(captured) => captured,
        Err(error) => return Err(error),
    };
    return Ok(serde_json::from_slice::<Value>(&out.stdout)
        .map(|meta| return collect_dep_names(&meta))
        .unwrap_or_default());
}

/// Matches root-relative generated globs, with recursive directory wildcards.
fn generated_glob_matches(declared: &str, path: &str) -> bool {
    let pattern = declared.strip_prefix("./").unwrap_or(declared);
    let Some(character) = pattern.chars().next() else {
        return path.is_empty();
    };
    let remaining = pattern.get(character.len_utf8()..).unwrap_or_default();
    if character == '*' {
        let recursive = remaining.starts_with('*');
        let suffix = remaining.strip_prefix('*').unwrap_or(remaining);
        if recursive
            && let Some(after_directory) = suffix.strip_prefix('/')
            && generated_glob_matches(after_directory, path)
        {
            return true;
        }
        return path
            .char_indices()
            .map(|(offset, _)| return offset)
            .chain(iter::once(path.len()))
            .take_while(|offset| {
                return recursive || !path.get(..*offset).unwrap_or_default().contains('/');
            })
            .any(|offset| {
                return generated_glob_matches(suffix, path.get(offset..).unwrap_or_default());
            });
    }
    let Some(first) = path.chars().next() else {
        return false;
    };
    return (character == first || (character == '?' && first != '/'))
        && generated_glob_matches(remaining, path.get(first.len_utf8()..).unwrap_or_default());
}

/// Whether the gate runs under CI (where the green-cache is bypassed so a fresh
/// run always validates).
fn in_ci() -> bool {
    return env::var("CI").is_ok() || env::var("GITHUB_ACTIONS").is_ok();
}

/// Attempts prebuilt installation, falling back to the original cargo install argv.
fn install_missing_tool(bin: &str, krate: &str, installer: &ToolInstaller) {
    let force: &[&str] = if resolve_tool(bin).is_some() {
        &[]
    } else {
        &["--force"]
    };
    let mut prebuilt = vec!["binstall", "--no-confirm"];
    prebuilt.extend_from_slice(force);
    prebuilt.push(krate);
    if matches!(installer.prebuilt, Prebuilt::Unavailable)
        || (installer.run)("cargo", &prebuilt) != ExitCode::SUCCESS
    {
        let mut install = vec!["install", "--locked"];
        install.extend_from_slice(force);
        install.push(krate);
        discard((installer.run)("cargo", &install));
    }
    return;
}

/// Whether a child tool's binary answers on the current path.
fn installed(bin: &str) -> bool {
    if !bin.starts_with("cargo-") && resolve_tool(bin).is_none() {
        return false;
    }
    return installed_command(bin)
        .tool_output()
        .is_ok_and(|out| return out.status.success());
}

/// Builds a version probe using cargo's subcommand convention when applicable.
fn installed_command(bin: &str) -> Command {
    let mut command = bin.strip_prefix("cargo-").map_or_else(
        || return tool_command(bin),
        |subcommand| {
            let mut cargo = tool_command("cargo");
            discard(cargo.arg(subcommand));
            return cargo;
        },
    );
    discard(command.arg("--version"));
    return command;
}

/// Whether the file is the embedded dprint.json with only plugin versions changed.
fn is_bumped_dprint(path: &Path, embedded: &str) -> bool {
    return fs::read_to_string(path)
        .is_ok_and(|content| return normalize_dprint(&content) == normalize_dprint(embedded));
}

/// Checks if file content matches expected embedded content.
fn is_lintmax_content(path: &Path, expected: &str) -> bool {
    return fs::read_to_string(path).map_or(true, |content| return content == expected);
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

/// Whether a candidate has a shell extension or a shell shebang.
fn is_shell(path: &Path) -> bool {
    return path
        .extension()
        .is_some_and(|extension| return extension == "sh")
        || fs::read_to_string(path).is_ok_and(|text| return shell_shebang(&text));
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

/// Escapes a literal filename for dprint's glob arguments.
fn literal_glob(text: &str) -> String {
    let mut pattern = String::new();
    for character in text.chars() {
        if matches!(character, '*' | '?' | '[' | ']' | '{' | '}' | '!' | '\\') {
            pattern.push('\\');
        }
        pattern.push(character);
    }
    return pattern;
}

/// Loads both exception files, requiring the explicitly selected file to be readable.
///
/// # Errors
/// Returns the underlying error if the required file or child process cannot be read or started.
fn load_project_exceptions() -> io::Result<Exceptions> {
    let external = env::var_os("LINTMAX_EXCEPTIONS").map(PathBuf::from);
    return read_exceptions(Path::new(EXCEPTIONS_FILE), external.as_deref());
}

/// Builds the bare machete invocation without an extra subcommand argument.
fn machete_command(paths: &[PathBuf]) -> Command {
    let mut command = tool_command("cargo-machete");
    discard(command.args(paths));
    return command;
}

/// Entry point.
fn main() -> ExitCode {
    let Cargo::Lintmax(cli) = Cargo::parse();

    return dispatch(cli.command.as_ref(), run_default, run_fix);
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

/// Extends spelling exclusions and vocabulary while preserving the embedded checks.
fn merge_typos_exceptions(content: &str, declared: &Exceptions) -> String {
    let excluded = declared
        .generated
        .iter()
        .map(|path| return format!("{}, ", toml::Value::String(path.clone())))
        .collect::<String>();
    let mut merged = content.replacen(
        "extend-exclude = [",
        &format!("extend-exclude = [{excluded}"),
        1,
    );
    if !declared.words.is_empty() {
        merged.push_str("\n[default.extend-words]\n");
        let unique: BTreeSet<&String> = declared.words.iter().collect();
        for word in unique {
            let quoted = toml::Value::String(word.clone()).to_string();
            merged.push_str(&quoted);
            merged.push_str(" = ");
            merged.push_str(&quoted);
            merged.push('\n');
        }
    }
    return merged;
}

/// Resolves the nightly rustfmt binary path used for strict (nightly-only) options.
fn nightly_rustfmt() -> Option<String> {
    let Ok(out) = tool_command("rustup")
        .args(["which", "--toolchain", "nightly", "rustfmt"])
        .tool_output()
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

/// Drops the `-<version>` segment from every dprint plugin URL so a bumped
/// config compares equal to the embedded seed.
fn normalize_dprint(text: &str) -> String {
    return text
        .lines()
        .map(normalize_dprint_line)
        .collect::<Vec<_>>()
        .join("\n");
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

/// Records the current tree-hash as the cwd's last green run.
fn persist_green(hash: Option<&String>) {
    if let (Some(digest), Some(cwd)) = (hash, state::cwd_key()) {
        let mut st = state::load();
        discard(st.last_green_by_cwd.insert(cwd, digest.clone()));
        st.save();
    }
    return;
}

/// Cargo package version, baked in at compile time.
const fn pkg_version() -> &'static str {
    return env!("CARGO_PKG_VERSION");
}

/// Ensures every check dependency before cache lookup or gate stages.
fn prepare_check(components: fn(), tools: fn(), rustfmt: fn()) {
    components();
    tools();
    rustfmt();
    return;
}

/// Writes configs before any refresh can ask dprint to update them.
fn prepare_fix(write: fn(), refresh: fn()) {
    write();
    refresh();
    return;
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
    discard(writeln!(
        out,
        "rustc forbid (binary targets only): dead_code_pub_in_binary"
    ));
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
    return;
}

/// Reads validated exceptions for config generation.
fn project_exceptions() -> Exceptions {
    return load_project_exceptions().unwrap_or_else(|error| {
        discard(exception_failure(&error));
        return Exceptions::default();
    });
}

/// Eligible project files, excluding vendored directories and generated globs.
///
/// # Errors
/// Returns an error if the root or eligible file paths cannot be read.
fn project_files() -> io::Result<Vec<PathBuf>> {
    let root = match env::current_dir() {
        Ok(directory) => directory,
        Err(error) => return Err(error),
    };
    let paths = match files::required(&root) {
        Ok(found) => found,
        Err(error) => return Err(error),
    };
    let vendored = match vendored_excludes() {
        Ok(found) => found,
        Err(error) => return Err(error),
    };
    let excludes: Vec<PathBuf> = vendored
        .iter()
        .map(|glob| return root.join(glob.strip_suffix("**").unwrap_or(glob)))
        .collect();
    let declared = match load_project_exceptions() {
        Ok(exceptions) => exceptions,
        Err(error) => return Err(error),
    };
    return Ok(filter_project_files(
        &root,
        paths,
        &excludes,
        &declared.generated,
    ));
}

/// Reads one exception source with path-bearing diagnostics.
///
/// # Errors
/// Returns an error for unreadable required files or invalid TOML.
fn read_exception_file(path: &Path, source: ExceptionSource) -> io::Result<Exceptions> {
    let text = match fs::read_to_string(path) {
        Ok(content) => content,
        Err(error)
            if matches!(source, ExceptionSource::Optional)
                && error.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(Exceptions::default());
        },
        Err(error) => return Err(exception_path_error(path, &error)),
    };
    return toml::from_str(&text).map_err(|error| {
        return io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {error}", path.display()),
        );
    });
}

/// Reads and merges root and external exceptions, preserving every list.
///
/// # Errors
/// Returns the underlying error if the required file or child process cannot be read or started.
fn read_exceptions(root: &Path, external: Option<&Path>) -> io::Result<Exceptions> {
    let mut merged = Exceptions::default();
    for declared in [
        read_exception_file(root, ExceptionSource::Optional),
        external.map_or_else(
            || return Ok(Exceptions::default()),
            |path| {
                return read_exception_file(path, ExceptionSource::Required);
            },
        ),
    ] {
        let parsed = match declared {
            Ok(value) => value,
            Err(error) => return Err(error),
        };
        merged.advisories.extend(parsed.advisories);
        merged.duplicates.extend(parsed.duplicates);
        merged.generated.extend(parsed.generated);
        merged.words.extend(parsed.words);
    }
    return Ok(merged);
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
    return;
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

/// Reports findings for shell entry points, retaining every severity and code.
fn report_shell_findings(findings: &[ShellFinding], scripts: &BTreeSet<PathBuf>) -> ExitCode {
    let mut result = ExitCode::SUCCESS;
    let selected = findings
        .iter()
        .filter(|finding| return scripts.contains(Path::new(&finding.file)));
    for finding in selected {
        discard(writeln!(
            io::stderr(),
            "{}:{}:{}: {}: {} [SC{}]",
            finding.file,
            finding.line,
            finding.column,
            finding.level,
            finding.message,
            finding.code
        ));
        result = ExitCode::FAILURE;
    }
    return result;
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

/// Resolves a binary from PATH, then the cargo installation directory.
fn resolve_tool(binary: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH");
    let cargo_home = env::var_os("CARGO_HOME");
    let home = env::var_os("HOME");
    return resolve_tool_in(
        binary,
        path.as_deref(),
        cargo_home.as_deref(),
        home.as_deref(),
    );
}

/// Resolves a tool using explicit environment values for isolated tests.
fn resolve_tool_in(
    binary: &str,
    path: Option<&OsStr>,
    cargo_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    let mut directories: Vec<PathBuf> = path
        .map(|value| return env::split_paths(value).collect())
        .unwrap_or_default();
    directories.extend(
        cargo_home
            .map(|cargo| return PathBuf::from(cargo).join("bin"))
            .or_else(|| {
                return home.map(|user_home| return PathBuf::from(user_home).join(".cargo/bin"));
            }),
    );
    for directory in directories {
        let candidate = directory.join(binary);
        if executable(&candidate) {
            return Some(candidate);
        }
    }
    return None;
}

/// Runs the in-house advisories, failing only when file enumeration fails.
fn run_advisories() -> ExitCode {
    return run_seq(&[
        ("staleness", run_staleness_advisory),
        ("dupconst", run_dupconst_advisory),
        ("gibberish", run_gibberish_advisory),
        ("floatdiv", run_floatdiv_advisory),
    ]);
}

/// Runs the binary lint only when workspace metadata contains a binary target.
fn run_binary_lint(mode: LintMode) -> ExitCode {
    return run_binary_lint_with(mode, workspace_has_binary(), cmd);
}

/// Preserves target probe and clippy failures while skipping library-only workspaces.
fn run_binary_lint_with(
    mode: LintMode,
    targets: io::Result<bool>,
    run: fn(&str, &[&str]) -> ExitCode,
) -> ExitCode {
    match targets {
        Ok(false) => return ExitCode::SUCCESS,
        Ok(true) => {},
        Err(error) => return enumeration_failure(&error),
    }
    let arguments = binary_lint_args(mode);
    let references: Vec<&str> = arguments.iter().map(String::as_str).collect();
    return run("cargo", &references);
}

/// Runs all checks with temporary configs (no green-cache; used by CI paths).
fn run_check_all() -> ExitCode {
    let configs = match check_configs() {
        Ok(directory) => directory,
        Err(error) => return exception_failure(&error),
    };
    discard(CHECK_CONFIGS.with(|directory| return directory.replace(Some(configs.clone()))));
    let result = run_seq(&[
        ("deny", run_deny),
        ("doc", run_doc),
        ("feature_matrix", run_feature_matrix),
        ("fmt_check", run_fmt_check),
        ("lint", run_lint),
        ("machete", run_machete),
        ("no_comments", run_no_comments),
        ("shellcheck", run_shellcheck),
        ("shfmt_check", run_shfmt_check),
        ("test", run_test),
        ("typos", run_typos),
    ]);
    let advisories = run_advisories();
    discard(CHECK_CONFIGS.with(|directory| return directory.replace(None)));
    discard(fs::remove_dir_all(configs));
    return worst(result, advisories);
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
    let result = cmd("cargo", &refs);
    if result != ExitCode::SUCCESS {
        return result;
    }
    return run_binary_lint(LintMode::Fix);
}

/// Default gate: short-circuits with `ok (cached)` when the working tree is
/// unchanged since the last clean run, otherwise runs the full gate and records
/// the green tree-hash on success.
fn run_default() -> ExitCode {
    if let Err(error) = load_project_exceptions() {
        return exception_failure(&error);
    }
    prepare_check(
        ensure_active_components,
        ensure_tools,
        ensure_nightly_rustfmt,
    );
    let key = match cache_key() {
        Ok(hash) => hash,
        Err(error) => return enumeration_failure(&error),
    };
    if cached_green(key.as_ref()) {
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

/// Runs cargo-deny; suppresses only upstream-transitive duplicates the project cannot fix.
fn run_deny() -> ExitCode {
    let mut command = tool_command("cargo");
    discard(command.args(["deny", "-L", "error"]));
    if checking() {
        discard(
            command
                .arg("--locked")
                .arg("--config")
                .arg(config_path("deny.toml")),
        );
    }
    discard(command.arg("check"));
    let output = match command.tool_output() {
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

/// Runs doctests, gating on real failures. A crate with no library target has no
/// doctests to run, which is success rather than an error.
fn run_doctests() -> ExitCode {
    let mut command = tool_command("cargo");
    command_args(&mut command, "cargo", &[
        "test",
        "--workspace",
        "--doc",
        "--all-features",
        "--quiet",
    ]);
    let output = command.tool_output();
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

/// Runs dprint on explicit eligible files, keeping configured exclusions.
fn run_dprint(action: &str) -> ExitCode {
    let paths = match project_files() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    if paths.is_empty() {
        return ExitCode::SUCCESS;
    }
    let patterns = match dprint_patterns(&paths) {
        Ok(found) => found,
        Err(error) => {
            emit(&error.to_string());
            return ExitCode::FAILURE;
        },
    };
    let status = dprint_command(action, &patterns).tool_status();
    return if status.is_ok_and(|completed| return completed.success()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
}

/// Reports duplicate-value const groups.
fn run_dupconst_advisory() -> ExitCode {
    let dups = match analyze::dupconst() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    advisory(
        &format!(
            "{} duplicate-value const group(s) (collapse to one):\n",
            dups.len()
        ),
        &analyze::format_dupconst(&dups),
    );
    return ExitCode::SUCCESS;
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

/// Auto-fixes clippy, comments, typos, and formatting.
fn run_fix() -> ExitCode {
    if let Err(error) = load_project_exceptions() {
        return exception_failure(&error);
    }
    ensure_active_components();
    ensure_tools();
    prepare_fix(write_configs, refresh_toolchain);
    let fixed = run_seq(&[
        ("clippy_fix", run_clippy_fix),
        ("reorder_items", run_reorder_items),
        ("remove_comments", run_remove_comments),
        ("typos_fix", run_typos_fix),
        ("shfmt_fix", run_shfmt_fix),
        ("fmt_all", run_fmt_all),
    ]);
    clean_configs();
    if fixed != ExitCode::SUCCESS {
        return fixed;
    }
    return run_check_all();
}

/// Reports unguarded float divisions.
fn run_floatdiv_advisory() -> ExitCode {
    let fdiv = match analyze::floatdiv() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    advisory(
        &format!(
            "{} unguarded float-division site(s) (NaN/Inf risk on empty input):\n",
            fdiv.len()
        ),
        &analyze::format_floatdiv(&fdiv),
    );
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

/// Formats explicit eligible Rust files without recursively loading modules.
fn run_fmt_members(rustfmt: &str, extra: &[&str]) -> ExitCode {
    let eligible = match project_files() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    let paths: Vec<PathBuf> = eligible
        .into_iter()
        .filter(|path| {
            return path
                .extension()
                .is_some_and(|extension| return extension == "rs");
        })
        .collect();
    if paths.is_empty() {
        return ExitCode::SUCCESS;
    }
    let mut result = ExitCode::SUCCESS;
    for path in paths {
        let mut command = tool_command(rustfmt);
        if checking() {
            discard(
                command
                    .arg("--config-path")
                    .arg(config_path("rustfmt.toml")),
            );
        }
        let status = command
            .args(extra)
            .args(["--config", "skip_children=true"])
            .arg(path)
            .tool_status();
        if !status.is_ok_and(|completed| return completed.success()) {
            result = ExitCode::FAILURE;
        }
    }
    return result;
}

/// Reports hash-gibberish identifiers.
fn run_gibberish_advisory() -> ExitCode {
    let found = match analyze::gibberish() {
        Ok(names) => names,
        Err(error) => return enumeration_failure(&error),
    };
    advisory("", &analyze::format_gibberish(&found));
    return ExitCode::SUCCESS;
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
    let result = cmd("cargo", &refs);
    if result != ExitCode::SUCCESS {
        return result;
    }
    return run_binary_lint(LintMode::Check);
}

/// Runs cargo-machete on explicit eligible manifests rather than a directory.
fn run_machete() -> ExitCode {
    let eligible = match project_files() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    let paths: Vec<PathBuf> = eligible
        .into_iter()
        .filter(|path| {
            return path
                .file_name()
                .is_some_and(|name| return name == "Cargo.toml");
        })
        .collect();
    if paths.is_empty() {
        return ExitCode::SUCCESS;
    }
    let output = machete_command(&paths).tool_output();
    return match output {
        Ok(out) if out.status.success() => ExitCode::SUCCESS,
        Ok(out) => {
            discard(io::stdout().write_all(&out.stdout));
            discard(io::stderr().write_all(&out.stderr));
            ExitCode::FAILURE
        },
        Err(_) => ExitCode::FAILURE,
    };
}

/// Checks that no non-survivor `//` comments exist in rust source.
fn run_no_comments() -> ExitCode {
    let mut found = false;
    let paths = match source_files() {
        Ok(eligible) => eligible,
        Err(error) => return enumeration_failure(&error),
    };
    for path in paths {
        if let Ok(content) = fs::read_to_string(&path) {
            found |= report_comments(&path, &content);
        }
    }
    if found {
        return ExitCode::FAILURE;
    }
    return ExitCode::SUCCESS;
}

/// Removes non-survivor `//` comments from rust source files.
fn run_remove_comments() -> ExitCode {
    let paths = match source_files() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    for path in paths {
        if let Ok(content) = fs::read_to_string(&path)
            && let Some(joined) = strip_content(&content)
        {
            discard(fs::write(&path, joined));
        }
    }
    return ExitCode::SUCCESS;
}

/// Sorts every module's items into the groups the ordering lint declares.
///
/// The rewrite refuses itself whenever a file's token multiset would change, so
/// a misparse leaves the file untouched rather than losing a declaration.
fn run_reorder_items() -> ExitCode {
    return match reorder::sort_tree(Path::new(".")) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => enumeration_failure(&error),
    };
}

/// Runs steps sequentially, stopping on first failure.
fn run_seq(steps: &[Stage<'_>]) -> ExitCode {
    return run_seq_to(steps, &mut io::stderr());
}

/// Runs named steps and writes a diagnostic for the first failure.
fn run_seq_to(steps: &[Stage<'_>], output: &mut impl io::Write) -> ExitCode {
    for &(name, step) in steps {
        let code = step();
        if code != ExitCode::SUCCESS {
            discard(writeln!(output, "lintmax: gate stage {name} failed"));
            return code;
        }
    }
    return ExitCode::SUCCESS;
}

/// Lints shell scripts while allowing sources only from the candidate set.
///
/// `ShellCheck` follows sourced files supplied as inputs without `-x`. Supplying
/// all candidates lets it read extensionless libraries too; only script entry
/// points contribute standalone findings, so other file types are not linted.
fn run_shellcheck() -> ExitCode {
    let candidates = match files::required(Path::new(".")) {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    let scripts: BTreeSet<PathBuf> = candidates
        .iter()
        .filter(|path| return is_shell(path))
        .cloned()
        .collect();
    if scripts.is_empty() {
        return ExitCode::SUCCESS;
    }
    return shellcheck_candidates(&candidates, &scripts);
}

/// Runs shfmt over every shell script with the given mode flag.
fn run_shfmt(mode: &str) -> ExitCode {
    let files = match shell_files() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
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

/// Verifies every shell script is shfmt-formatted (shell auto-detected from the
/// shebang, 2-space indent).
fn run_shfmt_check() -> ExitCode {
    return run_shfmt("-d");
}

/// Formats every shell script in place with shfmt.
fn run_shfmt_fix() -> ExitCode {
    return run_shfmt("-w");
}

/// Reports outdated dependencies without making outdated pins a gate failure.
fn run_staleness_advisory() -> ExitCode {
    let stale = match staleness::scan(Path::new("."), &project_exceptions().generated) {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    advisory(
        &format!(
            "{} dep(s) behind latest (bump toward active-maintenance window):\n",
            stale.len()
        ),
        &staleness::format(&stale),
    );
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

/// Checks for typos in source.
fn run_typos() -> ExitCode {
    return run_typos_excluded(&[]);
}

/// Runs typos on explicit eligible files, keeping configured exclusions.
fn run_typos_excluded(extra: &[&str]) -> ExitCode {
    let paths = match project_files() {
        Ok(found) => found,
        Err(error) => return enumeration_failure(&error),
    };
    if paths.is_empty() {
        return ExitCode::SUCCESS;
    }
    let status = typos_command(extra, &paths).tool_status();
    return if status.is_ok_and(|completed| return completed.success()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
}

/// Auto-fixes typos in source.
fn run_typos_fix() -> ExitCode {
    return run_typos_excluded(&["-w"]);
}

/// The rustfmt config written for the project; the base config verbatim.
///
/// Vendored crates are excluded by formatting workspace members explicitly
/// (`cargo fmt -p`), not by a rustfmt `ignore` block — an appended block also
/// makes the written `rustfmt.toml` itself dprint-dirty under a strict config.
fn rustfmt_with_ignores() -> String {
    return RUSTFMT_TOML.to_owned();
}

/// Eligible shell scripts by extension or shell shebang.
///
/// # Errors
/// Returns an error if eligible file enumeration fails.
fn shell_files() -> io::Result<Vec<PathBuf>> {
    return files::required(Path::new(".")).map(|paths| {
        return paths
            .into_iter()
            .filter(|path| return is_shell(path))
            .collect();
    });
}

/// Whether the first line names the `sh` interpreter.
fn shell_shebang(text: &str) -> bool {
    let Some(line) = text.lines().next() else {
        return false;
    };
    return line.starts_with("#!")
        && line
            .split(|character: char| return !character.is_alphanumeric() && character != '_')
            .any(|word| return word == "sh");
}

/// Runs `ShellCheck` with every eligible source pathname in its input allowlist.
fn shellcheck_candidates(candidates: &[PathBuf], scripts: &BTreeSet<PathBuf>) -> ExitCode {
    let output = match tool_command("shellcheck")
        .args(["--severity=style", "--enable=all", "--format=json"])
        .args(candidates)
        .tool_output()
    {
        Ok(result) => result,
        Err(error) => {
            emit(&format!("cannot run shellcheck: {error}"));
            return ExitCode::FAILURE;
        },
    };
    discard(io::stderr().write_all(&output.stderr));
    if !matches!(output.status.code(), Some(0_i32 | 1_i32)) {
        return ExitCode::FAILURE;
    }
    let findings: Vec<ShellFinding> = match serde_json::from_slice(&output.stdout) {
        Ok(decoded) => decoded,
        Err(error) => {
            emit(&format!("cannot read shellcheck findings: {error}"));
            return ExitCode::FAILURE;
        },
    };
    return report_shell_findings(&findings, scripts);
}

/// Source files scanned for comments (eligible `.rs` files under `src/`).
///
/// # Errors
/// Returns an error if eligible source file enumeration fails.
fn source_files() -> io::Result<Vec<PathBuf>> {
    return analyze::source_files();
}

/// Prints a tool spawn failure without altering the operating system error.
fn spawn_failure(program: &OsStr, error: &io::Error, output: &mut impl io::Write) {
    discard(writeln!(
        output,
        "lintmax: could not start {}: {error}",
        program.to_string_lossy()
    ));
    return;
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

/// Duplicates safe to suppress: every cargo-deny error is a duplicate no first-party crate causes.
fn suppressible_duplicates(stderr: &str) -> Option<Vec<String>> {
    let dups = match duplicate_only_failures(stderr) {
        Some(found) => found,
        None => return None,
    };
    if dups.is_empty() {
        return None;
    }
    let first_party = match first_party_direct_deps() {
        Ok(names) => names,
        Err(_) => return None,
    };
    if dups
        .iter()
        .any(|name| return first_party.contains(name.as_str()))
    {
        return None;
    }
    return Some(dups);
}

/// Builds every child command through the same binary resolver.
fn tool_command(binary: &str) -> Command {
    return Command::new(resolve_tool(binary).unwrap_or_else(|| return PathBuf::from(binary)));
}

/// Builds spelling commands that honor exclusions even for explicit filenames.
fn typos_command(extra: &[&str], paths: &[PathBuf]) -> Command {
    let mut command = tool_command("typos");
    discard(command.args(extra).arg("--force-exclude").args(paths));
    if checking() {
        discard(command.arg("--config").arg(config_path("typos.toml")));
    }
    return command;
}

/// Folds both exception sources into the temporary spelling configuration.
fn typos_with_exceptions(content: &str) -> String {
    return merge_typos_exceptions(content, &project_exceptions());
}

/// Exclude globs for vendored external packages (local non-workspace crates).
///
/// Like registry dependencies, vendored crates are compiled but not linted;
/// detected via cargo metadata, never by directory name, so no first-party
/// source can be excluded by accident.
///
/// # Errors
/// Returns the underlying error if the required file or child process cannot be read or started.
fn vendored_excludes() -> io::Result<Vec<String>> {
    let mut command = tool_command("cargo");
    discard(command.args(["metadata", "--format-version", "1"]));
    if checking() {
        discard(command.arg("--locked"));
    }
    let output = command.tool_output();
    let out = match output {
        Ok(captured) => captured,
        Err(error) => return Err(error),
    };
    let Ok(meta) = serde_json::from_slice::<Value>(&out.stdout) else {
        return Ok(Vec::new());
    };
    let members = workspace_member_ids(&meta);
    let cwd = env::current_dir().unwrap_or_default();
    let packages = meta.get("packages").and_then(Value::as_array);
    return Ok(packages
        .into_iter()
        .flatten()
        .filter(|pkg| return is_vendored_package(pkg, &members))
        .filter_map(|pkg| return package_dir_glob(pkg, &cwd))
        .collect());
}

/// Whether workspace metadata names any first-party binary target.
///
/// # Errors
/// Returns metadata command, status, or JSON decoding failures.
fn workspace_has_binary() -> io::Result<bool> {
    let mut command = tool_command("cargo");
    discard(command.args(["metadata", "--no-deps", "--format-version", "1"]));
    if checking() {
        discard(command.arg("--locked"));
    }
    let output = match command.tool_output() {
        Ok(captured) => captured,
        Err(error) => return Err(error),
    };
    if !output.status.success() {
        discard(io::stdout().write_all(&output.stdout));
        discard(io::stderr().write_all(&output.stderr));
        return Err(io::Error::other(
            "cargo metadata failed while selecting binary targets",
        ));
    }
    return serde_json::from_slice::<Value>(&output.stdout)
        .map(|metadata| return workspace_metadata_has_binary(&metadata))
        .map_err(io::Error::other);
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

/// Whether a workspace member declares a binary, excluding dependency targets.
fn workspace_metadata_has_binary(metadata: &Value) -> bool {
    let members = workspace_member_ids(metadata);
    return metadata
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|package| {
            return package
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|identifier| return members.contains(identifier));
        })
        .filter_map(|package| return package.get("targets").and_then(Value::as_array))
        .flatten()
        .filter_map(|target| return target.get("kind").and_then(Value::as_array))
        .flatten()
        .any(|kind| return kind.as_str() == Some("bin"));
}

/// Returns the worse of two exit codes.
fn worst(first: ExitCode, second: ExitCode) -> ExitCode {
    if first != ExitCode::SUCCESS {
        return first;
    }
    return second;
}

/// Writes each isolated config, propagating the first write failure.
///
/// # Errors
/// Returns an error if a config cannot be written.
fn write_check_configs(directory: &Path) -> io::Result<()> {
    for &(name, content) in MANAGED_CONFIGS {
        if let Err(error) = fs::write(directory.join(name), check_content(name, content)) {
            return Err(error);
        }
    }
    return Ok(());
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
    return;
}

/// Writes all temporary config files, then bumps dprint plugins to latest so
/// the embedded version pins are only a bootstrap seed, never a stale lock.
fn write_configs() {
    for &(name, content) in MANAGED_CONFIGS {
        if matches!(name, "dprint.json" | "typos.toml") {
            write_config(name, &check_content(name, content));
        } else {
            write_config(name, content);
        }
    }
    bump_dprint_plugins();
    return;
}

#[cfg(test)]
mod tests;
