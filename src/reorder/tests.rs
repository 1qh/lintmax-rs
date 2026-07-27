use std::{env, fs, path::PathBuf, process};

use super::{sort_file, tokens};

/// A file this run owns, so two runs never share a fixture path.
fn fixture(name: &str, body: &str) -> PathBuf {
    let path = env::temp_dir().join(format!("lintmax-reorder-{}-{name}.rs", process::id()));
    drop(fs::write(&path, body));
    return path;
}

/// # Panics
/// On assertion failure.
#[test]
fn a_function_declared_before_a_type_moves_below_it() {
    let path = fixture(
        "grouping",
        "use core::fmt;\n\nfn helper() {}\n\nstruct Held {\n    field: usize,\n}\n",
    );
    assert!(
        sort_file(&path),
        "the file is out of order, so it is rewritten"
    );
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    let struct_at = sorted.find("struct Held");
    let helper_at = sorted.find("fn helper");
    assert!(
        struct_at < helper_at && struct_at.is_some(),
        "the type sorts above the function the lint groups below it"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_file_already_in_order_is_left_alone() {
    let path = fixture(
        "ordered",
        "use core::fmt;\n\nstruct Held;\n\nfn helper() {}\n",
    );
    assert!(
        !sort_file(&path),
        "a file whose groups already ascend needs no rewrite"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_cfg_gated_item_holds_its_place() {
    let path = fixture(
        "gated",
        "use core::fmt;\n\nfn helper() {}\n\nstruct Held;\n\n#[cfg(test)]\nmod tests;\n",
    );
    let _sorted = sort_file(&path);
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    let gated_at = sorted.find("mod tests;");
    let helper_at = sorted.find("fn helper");
    assert!(
        gated_at > helper_at && gated_at.is_some(),
        "a gated item keeps the slot it was written in rather than sorting to the module group"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn the_guard_reads_a_moved_comma_as_no_change() {
    assert_eq!(
        tokens("struct Held { a: usize, b: usize }"),
        tokens("struct Held { b: usize, a: usize }"),
        "punctuation is excluded, so a reordered field's comma does not read as a lost token"
    );
}

/// # Panics
/// On assertion failure.
#[test]
fn the_guard_reads_a_dropped_name_as_a_change() {
    assert_ne!(
        tokens("struct Held { a: usize, b: usize }"),
        tokens("struct Held { a: usize }"),
        "a lost declaration changes the multiset, which is what the guard refuses on"
    );
}

/// # Panics
/// On assertion failure.
#[test]
fn a_const_function_sorts_as_a_function_not_a_constant() {
    let path = fixture(
        "constfn",
        "use core::fmt;\n\nconst fn helper() -> usize {\n    return 1;\n}\n\nstruct Held;\n",
    );
    assert!(
        sort_file(&path),
        "a const function declared above a type is out of order"
    );
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        sorted.find("struct Held") < sorted.find("const fn helper"),
        "a const function belongs to the function group, never the constant group"
    );
    drop(fs::remove_file(&path));
}
