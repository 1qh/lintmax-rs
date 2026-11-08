use core::{
    cell::RefCell,
    sync::atomic::{AtomicU64, Ordering},
};
use std::{
    env,
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process,
    process::ExitCode,
};

use super::normalize_dprint;

thread_local! {
    /// Calls recorded by isolated dispatch and preparation tests.
    static CALLS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// # Panics
/// On assertion failure.
#[test]
fn a_declared_advisory_lands_under_the_advisories_section() {
    let merged = merged_sample();
    let advisory_at = merged.find("RUSTSEC-2026-0192");
    assert!(advisory_at.is_some(), "the declared advisory is present");
    assert!(
        advisory_at > merged.find("[advisories]"),
        "it sits under [advisories]"
    );
    assert!(
        advisory_at < merged.find("[bans]"),
        "it must not fall into [bans]"
    );
    return;
}

/// # Panics
/// On assertion failure.
#[test]
fn a_declared_duplicate_lands_under_the_bans_section() {
    let merged = merged_sample();
    let duplicate_at = merged.find("ttf-parser@0.25.1");
    assert!(duplicate_at.is_some(), "the declared duplicate is present");
    assert!(duplicate_at > merged.find("[bans]"), "it sits under [bans]");
    return;
}

/// # Panics
/// On assertion failure.
#[test]
fn a_declared_generated_path_reaches_the_formatter_excludes() {
    let base = include_str!("../configs/dprint.json");
    assert!(
        base.contains("\"excludes\": [\n"),
        "the embedded config carries the excludes block this fold anchors on"
    );
    return;
}

/// # Panics
/// On assertion failure.
#[test]
fn a_project_with_no_exceptions_gets_the_embedded_config_unchanged() {
    let embedded = super::DENY_TOML;
    assert_eq!(
        super::merge_exceptions(embedded, &super::Exceptions::default()),
        embedded
    );
    return;
}

/// A missing prebuilt installer never receives an attempted install command.
///
/// # Panics
/// On assertion failure.
#[test]
fn absent_binstall_skips_straight_to_cargo_install() {
    CALLS.with(|calls| return calls.borrow_mut().clear());
    super::ensure_tool_with(
        "lintmax-test-missing-tool",
        "test-crate",
        &super::ToolInstaller {
            prebuilt: super::Prebuilt::Unavailable,
            probe: absent_tool,
            run: record_install,
        },
    );
    CALLS.with(|calls| {
        assert_eq!(*calls.borrow(), [
            "cargo install --locked --force test-crate"
        ]);
        return;
    });
    return;
}

/// Reports a missing fixture tool without probing the host.
fn absent_tool(_binary: &str) -> bool {
    return false;
}

/// # Panics
/// On assertion failure.
#[test]
fn an_undeclared_advisory_is_still_caught() {
    let declared = super::Exceptions {
        generated: Vec::new(),
        advisories: vec!["RUSTSEC-2026-0192".to_owned()],
        duplicates: vec![],
    };
    let merged = super::merge_exceptions(super::DENY_TOML, &declared);
    assert!(
        !merged.contains("RUSTSEC-2020-0000"),
        "only the declared advisories are ignored, so the gate still fails on any other"
    );
    return;
}

/// # Panics
/// On assertion failure.
#[test]
fn an_unknown_key_keeps_the_exceptions_a_project_already_declared() {
    let text = "advisories = [\"RUSTSEC-0000-0000\"]\nfrom-a-newer-gate = [\"x\"]\n";
    let parsed = toml::from_str::<super::Exceptions>(text).ok();
    assert_eq!(
        parsed.map(|held| return held.advisories),
        Some(vec!["RUSTSEC-0000-0000".to_owned()]),
        "a key this version does not know must not drop the exceptions it does know, or a \
         consumer running an older gate fails its supply-chain stage on every declared advisory"
    );
    return;
}

/// Bare invocation and explicit fix share the same action and exit status.
///
/// # Panics
/// On assertion failure.
#[test]
fn bare_command_dispatches_exactly_like_fix() {
    assert_eq!(
        super::dispatch(None, check_action, fix_action),
        ExitCode::from(23)
    );
    assert_eq!(
        super::dispatch(Some(&super::Sub::Fix), check_action, fix_action),
        ExitCode::from(23)
    );
    return;
}

/// Cargo subcommand probes pass their name through cargo before the version flag.
///
/// # Panics
/// On assertion failure.
#[test]
fn cargo_hack_probe_uses_the_cargo_subcommand() {
    let command = super::installed_command("cargo-hack");
    assert_eq!(
        command.get_program(),
        super::tool_command("cargo").get_program()
    );
    assert_eq!(command.get_args().collect::<Vec<_>>(), [
        "hack",
        "--version"
    ]);
    return;
}

/// A check action sentinel that never starts a child.
fn check_action() -> ExitCode {
    return ExitCode::from(17);
}

/// Locked checking preserves every Rust lint argument after the cargo separator.
///
/// # Panics
/// On assertion failure.
#[test]
fn check_arguments_preserve_the_compiler_separator() {
    super::discard(super::CHECK_CONFIGS.with(|directory| {
        return directory.replace(Some(PathBuf::from("isolated-configs")));
    }));
    let mut command = super::tool_command("cargo");
    super::command_args(&mut command, "cargo", &[
        "clippy",
        "--quiet",
        "--",
        "-F",
        "missing_docs",
    ]);
    super::discard(super::CHECK_CONFIGS.with(|directory| return directory.replace(None)));
    assert_eq!(command.get_args().collect::<Vec<_>>(), [
        "clippy",
        "--quiet",
        "--locked",
        "--",
        "-F",
        "missing_docs"
    ]);
    return;
}

/// Checker config generation leaves the project directory untouched.
///
/// # Panics
/// On assertion failure.
#[test]
fn check_configs_are_isolated_from_the_project() {
    let before = managed_config_contents();
    let directory = require_success(super::check_configs());
    check_isolated_config_directory(&directory);
    super::discard(fs::remove_dir_all(directory));
    let after = managed_config_contents();
    assert_eq!(before, after);
    assert!(!super::checking());
    return;
}

/// Checks that every generated config lives outside the project directory.
///
/// # Panics
/// On assertion failure.
fn check_isolated_config_directory(directory: &Path) {
    let cwd = env::current_dir().unwrap_or_default();
    assert!(!directory.starts_with(cwd));
    for &(name, _) in super::MANAGED_CONFIGS {
        assert!(directory.join(name).is_file());
    }
    return;
}

/// Checking requests components, child tools, and nightly formatting in order.
///
/// # Panics
/// On assertion failure.
#[test]
fn check_requests_every_required_tool_before_stages() {
    CALLS.with(|calls| return calls.borrow_mut().clear());
    super::prepare_check(record_components, record_tools, record_rustfmt);
    CALLS.with(|calls| {
        assert_eq!(*calls.borrow(), ["components", "tools", "nightly rustfmt"]);
        return;
    });
    return;
}

/// Check and informational dispatch cannot run the fix action or its refresh.
///
/// # Panics
/// On assertion failure.
#[test]
fn check_version_and_rules_never_dispatch_fix() {
    assert_eq!(
        super::dispatch(Some(&super::Sub::Check), check_action, forbidden_action),
        ExitCode::from(17)
    );
    assert_eq!(
        super::dispatch(
            Some(&super::Sub::Version),
            forbidden_action,
            forbidden_action
        ),
        ExitCode::SUCCESS
    );
    assert_eq!(
        super::dispatch(Some(&super::Sub::Rules), forbidden_action, forbidden_action),
        ExitCode::SUCCESS
    );
    return;
}

/// External exception changes invalidate a previously green cache identity.
///
/// # Panics
/// On assertion failure.
#[test]
fn external_exception_contents_change_the_cache_identity() {
    let directory = test_directory("cache-exceptions");
    let external = directory.join("external.toml");
    require_success(fs::write(&external, "advisories = []"));
    let before = super::cache_version(Some(&external)).ok();
    require_success(fs::write(&external, "advisories = [\"CHANGED\"]"));
    let after = super::cache_version(Some(&external)).ok();
    assert!(before.is_some());
    assert!(after.is_some());
    assert_ne!(before, after);
    super::discard(fs::remove_dir_all(directory));
    return;
}

/// A fix action sentinel that never starts a child.
fn fix_action() -> ExitCode {
    return ExitCode::from(23);
}

/// Config writing always precedes the toolchain refresh.
///
/// # Panics
/// On assertion failure.
#[test]
fn fix_writes_configs_before_refreshing() {
    CALLS.with(|calls| return calls.borrow_mut().clear());
    super::prepare_fix(record_configs, record_refresh);
    CALLS.with(|calls| {
        assert_eq!(*calls.borrow(), ["configs", "refresh"]);
        return;
    });
    return;
}

/// A forbidden action sentinel for read-only dispatch.
fn forbidden_action() -> ExitCode {
    return ExitCode::FAILURE;
}

/// The machete command retains the bare binary argv convention.
///
/// # Panics
/// On assertion failure.
#[test]
fn machete_command_does_not_add_a_subcommand_argument() {
    let command = super::machete_command(&[PathBuf::from("Cargo.toml")]);
    assert_eq!(command.get_args().collect::<Vec<_>>(), [OsStr::new(
        "Cargo.toml"
    )]);
    return;
}

/// Captures existing project config bytes without changing any files.
fn managed_config_contents() -> Vec<Option<Vec<u8>>> {
    return super::MANAGED_CONFIGS
        .iter()
        .map(|&(name, _)| return fs::read(name).ok())
        .collect();
}

/// The merged config for one declared advisory and one declared duplicate.
fn merged_sample() -> String {
    let declared = super::Exceptions {
        generated: Vec::new(),
        advisories: vec!["RUSTSEC-2026-0192".to_owned()],
        duplicates: vec!["ttf-parser@0.25.1".to_owned()],
    };
    return super::merge_exceptions(super::DENY_TOML, &declared);
}

/// A failing named stage keeps its exit code and stops before the next stage.
///
/// # Panics
/// On assertion failure.
#[test]
fn named_stage_reports_name_and_preserves_exit_code() {
    let mut output = Vec::new();
    assert_eq!(
        super::run_seq_to(
            &[("machete", fix_action), ("unreached", forbidden_action)],
            &mut output,
        ),
        ExitCode::from(23)
    );
    assert_eq!(output, b"lintmax: gate stage machete failed\n");
    return;
}

/// # Panics
/// On assertion failure.
#[test]
fn normalize_strips_plugin_version() {
    let pinned = "\"https://plugins.dprint.dev/toml-0.7.0.wasm\",";
    let other = "\"https://plugins.dprint.dev/toml-0.9.9.wasm\",";
    assert_eq!(normalize_dprint(pinned), normalize_dprint(other));
    return;
}

/// Both exception sources contribute all three lists.
///
/// # Panics
/// On assertion failure.
#[test]
fn project_and_external_exceptions_merge_every_list() {
    let directory = test_directory("exceptions");
    let root = directory.join("root.toml");
    let external = directory.join("external.toml");
    require_success(fs::write(
        &root,
        "advisories = [\"ROOT\"]\nduplicates = [\"root@1\"]\ngenerated = [\"root.rs\"]\n",
    ));
    require_success(fs::write(
        &external,
        "advisories = [\"EXTERNAL\"]\nduplicates = [\"external@2\"]\ngenerated = \
         [\"external.rs\"]\n",
    ));
    let declared = require_success(super::read_exceptions(&root, Some(&external)));
    {
        assert_eq!(declared.advisories, ["ROOT", "EXTERNAL"]);
        assert_eq!(declared.duplicates, ["root@1", "external@2"]);
        assert_eq!(declared.generated, ["root.rs", "external.rs"]);
    }
    super::discard(fs::remove_dir_all(directory));
    return;
}

/// Records active-toolchain component preparation.
fn record_components() {
    CALLS.with(|calls| return calls.borrow_mut().push("components".to_owned()));
    return;
}

/// Records the configuration-writing step.
fn record_configs() {
    CALLS.with(|calls| return calls.borrow_mut().push("configs".to_owned()));
    return;
}

/// Records installer argv without invoking cargo or writing installed tools.
fn record_install(binary: &str, arguments: &[&str]) -> ExitCode {
    CALLS.with(|calls| {
        return calls
            .borrow_mut()
            .push(format!("{binary} {}", arguments.join(" ")));
    });
    return ExitCode::SUCCESS;
}

/// Records the toolchain-refresh step.
fn record_refresh() {
    CALLS.with(|calls| return calls.borrow_mut().push("refresh".to_owned()));
    return;
}

/// Records nightly rustfmt preparation without starting rustup.
fn record_rustfmt() {
    CALLS.with(|calls| return calls.borrow_mut().push("nightly rustfmt".to_owned()));
    return;
}

/// Records child-tool preparation.
fn record_tools() {
    CALLS.with(|calls| return calls.borrow_mut().push("tools".to_owned()));
    return;
}

/// Extracts a fixture result with its original failure diagnostic.
///
/// # Panics
/// When fixture setup or lookup fails.
fn require_success<T>(result: io::Result<T>) -> T
where
    T: Default,
{
    let (value, error) = match result {
        Ok(value) => (value, None),
        Err(error) => (T::default(), Some(error.to_string())),
    };
    assert_eq!(error, None, "fixture operation failed");
    return value;
}

/// An executable fixture may reuse this test executable without starting it.
///
/// # Panics
/// On fixture setup failure.
fn resolver_fixture(directory: &Path, name: &str) -> PathBuf {
    require_success(fs::create_dir_all(directory));
    let destination = directory.join(name);
    let executable = std_env_current_exe();
    super::discard(require_success(fs::copy(executable, &destination)));
    return destination;
}

/// Resolution prefers PATH, then `CARGO_HOME`, then the default cargo home.
///
/// # Panics
/// On assertion failure.
#[test]
fn resolver_searches_path_then_cargo_home_and_default_home() {
    let directory = test_directory("resolver");
    let path = directory.join("path");
    let cargo = directory.join("cargo");
    let home = directory.join("home");
    let path_binary = resolver_fixture(&path, "lintmax-test-tool");
    let cargo_binary = resolver_fixture(&cargo.join("bin"), "lintmax-test-tool");
    let default_binary = resolver_fixture(&home.join(".cargo/bin"), "lintmax-test-tool");
    assert_eq!(
        super::resolve_tool_in(
            "lintmax-test-tool",
            Some(path.as_os_str()),
            Some(cargo.as_os_str()),
            Some(home.as_os_str()),
        ),
        Some(path_binary)
    );
    assert_eq!(
        super::resolve_tool_in(
            "lintmax-test-tool",
            None,
            Some(cargo.as_os_str()),
            Some(home.as_os_str()),
        ),
        Some(cargo_binary)
    );
    assert_eq!(
        super::resolve_tool_in("lintmax-test-tool", None, None, Some(home.as_os_str())),
        Some(default_binary)
    );
    assert_eq!(super::resolve_tool_in("missing", None, None, None), None);
    super::discard(fs::remove_dir_all(directory));
    return;
}

/// The new stable allow-by-default lints are forbidden in sorted order.
///
/// # Panics
/// On assertion failure.
#[test]
fn rustc_forbid_includes_new_stable_lints_in_order() {
    stable_lint_flags_are_forbidden();
    assert!(
        super::RUSTC_FORBID
            .windows(2)
            .all(|pair| return pair.first() < pair.get(1))
    );
    unstable_lint_documentation_is_current();
    return;
}

/// Every spawn error names its binary and includes the original IO error.
///
/// # Panics
/// On assertion failure.
#[test]
fn spawn_errors_name_the_tool_and_io_error() {
    let error = io::Error::new(io::ErrorKind::NotFound, "fixture tool is missing");
    for binary in [
        "cargo",
        "cargo-machete",
        "dprint",
        "typos",
        "shellcheck",
        "shfmt",
        "rustfmt",
        "rustup",
    ] {
        let mut output = Vec::new();
        super::spawn_failure(OsStr::new(binary), &error, &mut output);
        assert_eq!(
            String::from_utf8_lossy(&output),
            format!("lintmax: could not start {binary}: {error}\n")
        );
    }
    assert_eq!(
        super::cmd("lintmax-test-no-such-command", &[]),
        ExitCode::FAILURE
    );
    assert_eq!(
        super::cmd_quiet("lintmax-test-no-such-command", &[]),
        ExitCode::FAILURE
    );
    return;
}

/// Checks the compiler arguments for each newly stable lint.
///
/// # Panics
/// On assertion failure.
fn stable_lint_flags_are_forbidden() {
    let arguments = super::build_lint_args();
    for lint in ["dead_code_pub_in_binary", "raw_borrows_via_references"] {
        assert!(arguments.windows(2).any(|pair| return pair == ["-F", lint]));
    }
    return;
}

/// Returns this test executable's path for executable resolver fixtures.
///
/// # Panics
/// When the operating system cannot locate the current executable.
fn std_env_current_exe() -> PathBuf {
    return require_success(env::current_exe());
}

/// Creates a uniquely named scratch directory outside the repository.
///
/// # Panics
/// On fixture setup failure.
fn test_directory(label: &str) -> PathBuf {
    /// Unique scratch-directory sequence within this process.
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let directory = env::temp_dir().join(format!(
        "lintmax-test-{label}-{}-{}",
        process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    require_success(fs::create_dir_all(&directory));
    return directory;
}

/// An explicitly selected unreadable exception file is a path-bearing error.
///
/// # Panics
/// On assertion failure.
#[test]
fn unreadable_external_exceptions_fail_with_the_path() {
    let directory = test_directory("unreadable");
    let external = directory.join("missing.toml");
    let result = super::read_exceptions(&directory.join("absent.toml"), Some(&external));
    let failure = match result {
        Ok(_) => None,
        Err(error) => Some(error.to_string()),
    };
    assert!(failure.is_some());
    assert!(
        failure
            .unwrap_or_default()
            .contains(external.to_string_lossy().as_ref())
    );
    super::discard(fs::remove_dir_all(directory));
    return;
}

/// Checks the documented unstable lint names against their current spellings.
///
/// # Panics
/// On assertion failure.
fn unstable_lint_documentation_is_current() {
    let documentation = include_str!("../CLAUDE.md");
    assert!(documentation.contains("`implicit_provenance_casts`"));
    assert!(documentation.contains("`deprecated_llvm_intrinsic`"));
    assert!(!documentation.contains("`fuzzy_provenance_casts`"));
    assert!(!documentation.contains("`lossy_provenance_casts`"));
    return;
}
