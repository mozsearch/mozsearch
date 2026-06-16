mod owned_or_borrowed;

#[allow(unused_imports)]
pub use owned_or_borrowed::OwnedOrBorrowed;

/// Compares strings in lexicographic order case-insensitively
///
/// ```
/// # use tools::utils::case_insensitive_cmp;
/// # use core::cmp::Ordering::*;
/// assert_eq!(case_insensitive_cmp("ABCD".chars(), "ABC".chars()), Greater);
/// assert_eq!(case_insensitive_cmp("ABC".chars(), "ABCD".chars()), Less);
/// assert_eq!(case_insensitive_cmp("ABC".chars(), "ABC".chars()), Equal);
/// assert_eq!(case_insensitive_cmp("ABC".chars(), "abc".chars()), Equal);
/// ```
pub fn case_insensitive_cmp<Lhs, Rhs>(lhs: Lhs, rhs: Rhs) -> core::cmp::Ordering
where
    Lhs: Iterator<Item = char>,
    Rhs: Iterator<Item = char>,
{
    let lhs_uppercase = lhs.flat_map(char::to_uppercase);
    let rhs_uppercase = rhs.flat_map(char::to_uppercase);
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
/// assert_eq!(case_semisensitive_cmp("ABCD".chars(), "ABC".chars()), Greater);
/// assert_eq!(case_semisensitive_cmp("ABC".chars(), "ABCD".chars()), Less);
/// assert_eq!(case_semisensitive_cmp("ABC".chars(), "ABC".chars()), Equal);
/// assert_eq!(case_semisensitive_cmp("ABC".chars(), "abc".chars()), Less);
/// ```
pub fn case_semisensitive_cmp<Lhs, Rhs>(lhs: Lhs, rhs: Rhs) -> core::cmp::Ordering
where
    Lhs: Iterator<Item = char> + Clone,
    Rhs: Iterator<Item = char> + Clone,
{
    case_insensitive_cmp(lhs.clone(), rhs.clone()).then_with(move || lhs.cmp(rhs))
}
