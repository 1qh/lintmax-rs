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

/// # Panics
/// On assertion failure.
#[test]
fn an_impl_blocks_members_sort_by_name() {
    let path = fixture(
        "impl-members",
        "struct Held;\n\nimpl Held {\n    fn zulu(&self) {}\n\n    fn alpha(&self) {}\n}\n",
    );
    assert!(super::sort_members(&path), "the members are out of order");
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        sorted.find("fn alpha") < sorted.find("fn zulu"),
        "an impl block's members sort by their own names"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_types_fields_sort_by_name() {
    let path = fixture(
        "type-members",
        "struct Held {\n    zulu: usize,\n    alpha: usize,\n}\n",
    );
    assert!(super::sort_members(&path), "the fields are out of order");
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        sorted.find("alpha") < sorted.find("zulu"),
        "a type's fields sort by their own names"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_block_already_in_order_is_left_alone() {
    let path = fixture(
        "ordered-members",
        "struct Held {\n    alpha: usize,\n    zulu: usize,\n}\n",
    );
    assert!(
        !super::sort_members(&path),
        "a block whose members already ascend needs no rewrite"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_brace_inside_a_literal_never_swallows_the_items_after_it() {
    let path = fixture(
        "literal-braces",
        "fn opener() -> char {\n    return '{';\n}\n\n/// Doc.\ntype Held = usize;\n",
    );
    assert!(
        sort_file(&path),
        "the type after a brace-carrying literal is an item the walk can still see"
    );
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        sorted.find("type Held") < sorted.find("fn opener"),
        "a type sorts above a function, which a raw-text depth walk never reaches"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_lifetime_is_never_read_as_a_character_literal() {
    assert_eq!(
        super::code_only("fn held(value: &'static str) -> char { '}' }"),
        "fn held(value: &'static str) -> char {  }",
        "a lifetime apostrophe keeps its braces countable while a literal loses its own"
    );
}

/// # Panics
/// On assertion failure.
#[test]
fn a_member_whose_signature_spans_lines_still_sorts() {
    let path =
        fixture(
            "wrapped-signature",
            "impl Held {\n    /// Doc.\n    fn zulu(&self) -> usize {\n        return 0;\n    \
             }\n\n    /// Doc.\n    const fn alpha<'held>(\n        &self,\n        value: &'held \
             str,\n    ) -> &'held str {\n        return value;\n    }\n}\n",
        );
    assert!(
        super::sort_members(&path),
        "a signature wrapped across lines is still a member the sort can move"
    );
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        sorted.find("fn alpha") < sorted.find("fn zulu"),
        "the wrapped member sorts above the one-line one"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_block_holding_a_line_no_member_owns_is_left_alone() {
    let path = fixture(
        "untiled-block",
        "impl Held {
    /// Doc.
    fn zulu(&self) -> usize {
        return 0;
    }
    held_macro!(zulu);

    /// Doc.
    fn alpha(&self) -> usize {
        return 1;
    }
}
",
    );
    let before = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        !super::sort_members(&path),
        "a block the members do not tile is never rewritten"
    );
    assert_eq!(
        fs::read_to_string(&path).unwrap_or_default(),
        before,
        "every byte survives, where the splice would have dropped the unowned line"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn an_argument_comma_never_ends_a_member_early() {
    let path =
        fixture(
            "wrapped-arguments",
            "impl Held {\n    /// Doc.\n    fn zulu(\n        &self,\n        value: usize,\n    \
             ) -> usize {\n        return value;\n    }\n\n    /// Doc.\n    fn alpha(&self) -> \
             usize {\n        return 1;\n    }\n}\n",
        );
    assert!(
        super::sort_members(&path),
        "a member whose arguments wrap is one member, so the block still tiles"
    );
    let sorted = fs::read_to_string(&path).unwrap_or_default();
    assert!(
        sorted.find("fn alpha") < sorted.find("fn zulu"),
        "the wrapped member moves whole rather than splitting at its argument comma"
    );
    assert!(
        sorted.contains("value: usize,"),
        "its arguments travel with it"
    );
    drop(fs::remove_file(&path));
}

/// # Panics
/// On assertion failure.
#[test]
fn a_member_left_with_no_doc_is_named() {
    let lines: Vec<String> = vec![
        "impl Held {".to_owned(),
        "    /// Doc.".to_owned(),
        "    fn alpha(&self) {}".to_owned(),
        String::new(),
        "    fn zulu(&self) {}".to_owned(),
        "}".to_owned(),
    ];
    assert_eq!(
        super::undocumented_member(&lines, 2),
        None,
        "a member carrying its own doc is not reported"
    );
    assert_eq!(
        super::undocumented_member(&lines, 4),
        Some("zulu".to_owned()),
        "the item a stranded doc left behind is the detectable half"
    );
}

/// # Panics
/// On assertion failure.
#[test]
fn an_attribute_between_a_doc_and_its_member_still_counts_as_documented() {
    let lines: Vec<String> = vec![
        "impl Held {".to_owned(),
        "    /// Doc.".to_owned(),
        "    #[must_use]".to_owned(),
        "    fn alpha(&self) {}".to_owned(),
        "}".to_owned(),
    ];
    assert_eq!(
        super::undocumented_member(&lines, 3),
        None,
        "an attribute sits between a doc and its item without detaching them"
    );
}
