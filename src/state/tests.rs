use super::fnv1a;

/// # Panics
/// On assertion failure.
#[test]
fn fnv1a_differs_on_change() {
    assert_ne!(fnv1a(b"alpha"), fnv1a(b"alpha2"));
}

/// # Panics
/// On assertion failure.
#[test]
fn fnv1a_stable() {
    assert_eq!(fnv1a(b"lintmax"), fnv1a(b"lintmax"));
}
