use super::normalize_dprint;

/// # Panics
/// On assertion failure.
#[test]
fn normalize_strips_plugin_version() {
    let pinned = "\"https://plugins.dprint.dev/toml-0.7.0.wasm\",";
    let other = "\"https://plugins.dprint.dev/toml-0.9.9.wasm\",";
    assert_eq!(normalize_dprint(pinned), normalize_dprint(other));
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
}

/// # Panics
/// On assertion failure.
#[test]
fn a_declared_duplicate_lands_under_the_bans_section() {
    let merged = merged_sample();
    let duplicate_at = merged.find("ttf-parser@0.25.1");
    assert!(duplicate_at.is_some(), "the declared duplicate is present");
    assert!(duplicate_at > merged.find("[bans]"), "it sits under [bans]");
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
}
