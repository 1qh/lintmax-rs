//! `cargo lintmax` — maximum strictness Rust pipeline in one command.

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

/// Embedded bacon configuration.
const BACON_TOML: &str = include_str!("../configs/bacon.toml");
/// Embedded clippy configuration.
const CLIPPY_TOML: &str = include_str!("../configs/clippy.toml");
/// Embedded cargo-deny configuration.
const DENY_TOML: &str = include_str!("../configs/deny.toml");
/// Embedded dprint configuration.
const DPRINT_JSON: &str = include_str!("../configs/dprint.json");
/// Embedded editorconfig.
const EDITORCONFIG: &str = include_str!("../configs/editorconfig");
/// Git pre-commit hook content.
const PRE_COMMIT: &str = "#!/bin/sh\ncargo lintmax\n";
/// Embedded rust-analyzer configuration.
const RUST_ANALYZER_TOML: &str = include_str!("../configs/rust-analyzer.toml");
/// Embedded rustfmt configuration.
const RUSTFMT_TOML: &str = include_str!("../configs/rustfmt.toml");
/// Embedded typos configuration.
const TYPOS_TOML: &str = include_str!("../configs/typos.toml");

/// GitHub Actions CI workflow.
const CI_YML: &str = "name: CI\n\
    on: [push, pull_request]\n\
    \n\
    env:\n\
    \x20 CARGO_TERM_COLOR: always\n\
    \x20 CARGO_INCREMENTAL: 0\n\
    \n\
    jobs:\n\
    \x20 ci:\n\
    \x20\x20\x20 runs-on: ubuntu-latest\n\
    \x20\x20\x20 steps:\n\
    \x20\x20\x20\x20\x20 - uses: actions/checkout@v6\n\
    \x20\x20\x20\x20\x20 - uses: dtolnay/rust-toolchain@stable\n\
    \x20\x20\x20\x20\x20\x20\x20 with:\n\
    \x20\x20\x20\x20\x20\x20\x20\x20\x20 components: clippy, rustfmt, llvm-tools-preview\n\
    \x20\x20\x20\x20\x20 - uses: Swatinem/rust-cache@v2\n\
    \x20\x20\x20\x20\x20 - uses: taiki-e/install-action@v2\n\
    \x20\x20\x20\x20\x20\x20\x20 with:\n\
    \x20\x20\x20\x20\x20\x20\x20\x20\x20 tool: cargo-nextest,cargo-deny,cargo-machete,cargo-llvm-cov,dprint,typos-cli,ripgrep,cargo-lintmax\n\
    \x20\x20\x20\x20\x20 - run: cargo lintmax ci\n\
    \x20\x20\x20\x20\x20 - run: cargo lintmax cov-ci\n\
    \x20\x20\x20\x20\x20 - uses: actions/upload-artifact@v7\n\
    \x20\x20\x20\x20\x20\x20\x20 with:\n\
    \x20\x20\x20\x20\x20\x20\x20\x20\x20 name: coverage\n\
    \x20\x20\x20\x20\x20\x20\x20\x20\x20 path: lcov.info\n";

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
    /// Full pipeline (clean, update, check all).
    Ci,
    /// CI pipeline (no clean).
    CiRemote,
    /// Coverage report (opens browser).
    Cov,
    /// Coverage for CI (lcov output).
    CovCi,
    /// Auto-fix everything.
    Fix,
    /// Format all files.
    Fmt,
    /// Setup git hooks and CI workflow.
    Init,
    /// Dev loop with bacon.
    Watch,
}

/// Discards a result, satisfying must-use and drop lints.
fn discard<T>(_value: T) {}

/// Removes temporary config files if they match embedded content.
fn clean_configs() {
    for (name, content) in MANAGED_CONFIGS {
        let path = config_path(name);
        if is_lintmax_content(&path, content) {
            discard(fs::remove_file(path));
        }
    }
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

/// Returns path for a config file name.
fn config_path(name: &str) -> PathBuf {
    return PathBuf::from(name);
}

/// Checks if file content matches expected embedded content.
fn is_lintmax_content(path: &Path, expected: &str) -> bool {
    return fs::read_to_string(path)
        .map(|content| return content == expected)
        .unwrap_or(true);
}

/// Entry point.
fn main() -> ExitCode {
    let Cargo::Lintmax(cli) = Cargo::parse();

    return match cli.command {
        None => run_check_all(),
        Some(Sub::Ci) => run_ci(),
        Some(Sub::CiRemote) => run_ci_remote(),
        Some(Sub::Cov) => run_cov(),
        Some(Sub::CovCi) => run_cov_ci(),
        Some(Sub::Fix) => run_fix(),
        Some(Sub::Fmt) => run_fmt(),
        Some(Sub::Init) => run_init(),
        Some(Sub::Watch) => run_watch(),
    };
}

/// Runs all checks with temporary configs.
fn run_check_all() -> ExitCode {
    write_configs();
    let result = run_seq(&[
        run_deny,
        run_doc,
        run_fmt_check,
        run_lint,
        run_machete,
        run_no_comments,
        run_test,
        run_typos,
    ]);
    clean_configs();
    return result;
}

/// Runs clean, update, then all checks.
fn run_ci() -> ExitCode {
    return run_seq(&[run_clean, run_update, run_check_all]);
}

/// Runs update then all checks (no clean).
fn run_ci_remote() -> ExitCode {
    return run_seq(&[run_update, run_check_all]);
}

/// Cleans build artifacts.
fn run_clean() -> ExitCode {
    return cmd("cargo", &["clean"]);
}

/// Runs clippy auto-fix.
fn run_clippy_fix() -> ExitCode {
    return cmd(
        "cargo",
        &[
            "clippy",
            "--all-targets",
            "--all-features",
            "--fix",
            "--allow-dirty",
            "--quiet",
        ],
    );
}

/// Opens coverage report in browser.
fn run_cov() -> ExitCode {
    return cmd("cargo", &["llvm-cov", "--all-features", "--open"]);
}

/// Generates lcov coverage for CI.
fn run_cov_ci() -> ExitCode {
    return cmd(
        "cargo",
        &[
            "llvm-cov",
            "--all-features",
            "--lcov",
            "--output-path",
            "lcov.info",
        ],
    );
}

/// Runs cargo-deny dependency audit.
fn run_deny() -> ExitCode {
    return cmd("cargo", &["deny", "-L", "error", "check"]);
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
        run_fmt_all,
    ]);
    clean_configs();
    return result;
}

/// Formats all files with temporary configs.
fn run_fmt() -> ExitCode {
    write_configs();
    let result = run_fmt_all();
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

/// Sets up git hooks, CI workflow, and editor configs.
fn run_init() -> ExitCode {
    discard(fs::create_dir_all(".githooks"));
    discard(fs::write(".githooks/pre-commit", PRE_COMMIT));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        discard(fs::set_permissions(
            ".githooks/pre-commit",
            fs::Permissions::from_mode(0o755),
        ))
    };
    discard(cmd("git", &["config", "core.hooksPath", ".githooks"]));
    discard(fs::create_dir_all(".github/workflows"));
    discard(fs::write(".github/workflows/ci.yml", CI_YML));
    discard(fs::write(".editorconfig", EDITORCONFIG));
    discard(fs::write("rust-analyzer.toml", RUST_ANALYZER_TOML));
    discard(writeln!(
        io::stderr(),
        "initialized: .githooks, .github/workflows/ci.yml, .editorconfig, rust-analyzer.toml"
    ));
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

    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    return cmd("cargo", &refs);
}

/// Runs cargo-machete unused dependency check.
fn run_machete() -> ExitCode {
    return cmd("cargo", &["machete"]);
}

/// Checks that no `//` comments exist in rust source.
fn run_no_comments() -> ExitCode {
    let output = Command::new("rg")
        .args(["--quiet", r"^\s*//[^/!]", "-t", "rust", "src/"])
        .status();
    return match output {
        Ok(status) if status.success() => {
            discard(writeln!(
                io::stderr(),
                "error: found // comments in source (only /// doc comments allowed)"
            ));
            ExitCode::FAILURE
        }
        _ => ExitCode::SUCCESS,
    };
}

/// Removes `//` comments from rust source files.
fn run_remove_comments() -> ExitCode {
    let output = Command::new("rg")
        .args(["-l", r"^\s*//[^/!]", "-t", "rust", "src/"])
        .output();
    if let Ok(out) = output {
        let files = String::from_utf8_lossy(&out.stdout);
        for file in files.lines().filter(|line| return !line.is_empty()) {
            discard(
                Command::new("perl")
                    .args(["-ni", "-e", r"print unless /^\s*\/\/[^\/!]/", file])
                    .status(),
            );
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
    let result = cmd(
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

/// Updates cargo deps and dprint plugins.
fn run_update() -> ExitCode {
    discard(cmd("cargo", &["update"]));
    discard(cmd("dprint", &["config", "update"]));
    return ExitCode::SUCCESS;
}

/// Starts bacon dev loop.
fn run_watch() -> ExitCode {
    write_config("bacon.toml", BACON_TOML);
    write_config("clippy.toml", CLIPPY_TOML);
    return cmd("bacon", &["clippy"]);
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

/// Writes all temporary config files.
fn write_configs() {
    for (name, content) in MANAGED_CONFIGS {
        write_config(name, content);
    }
}
