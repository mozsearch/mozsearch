mod owned_or_borrowed;

#[allow(unused_imports)]
pub use owned_or_borrowed::OwnedOrBorrowed;

/// Compares strings in lexicographic order case-insensitively
///
/// ```
/// # use tools::utils::case_insensitive_cmp;
/// # use core::cmp::Ordering::*;
/// assert_eq!(case_insensitive_cmp("ABCD", "ABC"), Greater);
/// assert_eq!(case_insensitive_cmp("ABC", "ABCD"), Less);
/// assert_eq!(case_insensitive_cmp("ABC", "ABC"), Equal);
/// assert_eq!(case_insensitive_cmp("ABC", "abc"), Equal);
/// ```
pub fn case_insensitive_cmp(lhs: impl AsRef<str>, rhs: impl AsRef<str>) -> core::cmp::Ordering {
    let lhs = lhs.as_ref();
    let rhs = rhs.as_ref();

    let lhs_uppercase = lhs.chars().flat_map(char::to_uppercase);
    let rhs_uppercase = rhs.chars().flat_map(char::to_uppercase);
    lhs_uppercase.cmp(rhs_uppercase)
}

/// Compares strings in lexicographic order:
/// - first case-insensitively
/// - then case-sensitively if they compare equal
///
/// This is meant to provide a more stable ordering than pure case-insensitive comparison.
///
/// ```
/// # use tools::utils::case_semisensitive_cmp;
/// # use core::cmp::Ordering::*;
/// assert_eq!(case_semisensitive_cmp("ABCD", "ABC"), Greater);
/// assert_eq!(case_semisensitive_cmp("ABC", "ABCD"), Less);
/// assert_eq!(case_semisensitive_cmp("ABC", "ABC"), Equal);
/// assert_eq!(case_semisensitive_cmp("ABC", "abc"), Less);
/// ```
pub fn case_semisensitive_cmp(lhs: impl AsRef<str>, rhs: impl AsRef<str>) -> core::cmp::Ordering {
    let lhs = lhs.as_ref();
    let rhs = rhs.as_ref();

    case_insensitive_cmp(lhs, rhs).then_with(|| lhs.cmp(rhs))
}
