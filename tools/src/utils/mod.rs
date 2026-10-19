mod overflowing_vec;
mod owned_or_borrowed;

#[allow(unused_imports)]
pub use owned_or_borrowed::OwnedOrBorrowed;

#[allow(unused_imports)]
pub use overflowing_vec::OverflowingVec;

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

/// `case_semisensitive_cmp` of two strings, by their bytes if they're ASCII
/// (whose uppercase is ASCII's, as `char::to_uppercase` has it, so the order is
/// the same), which is ~10 times faster than by their chars' uppercase (ex:
/// sorting firefox-disco's identifiers, single-threaded in crossref).
///
/// ```
/// # use tools::utils::{case_semisensitive_cmp, case_semisensitive_str_cmp};
/// for (a, b) in [("ABCD", "ABC"), ("abc", "ABC"), ("a_b", "aB"), ("é", "E"), ("x", "x")] {
///     assert_eq!(case_semisensitive_str_cmp(a, b), case_semisensitive_cmp(a.chars(), b.chars()));
/// }
/// ```
pub fn case_semisensitive_str_cmp(lhs: &str, rhs: &str) -> core::cmp::Ordering {
    if lhs.is_ascii() && rhs.is_ascii() {
        let (lhs, rhs) = (lhs.as_bytes(), rhs.as_bytes());
        lhs.iter()
            .map(u8::to_ascii_uppercase)
            .cmp(rhs.iter().map(u8::to_ascii_uppercase))
            .then_with(|| lhs.cmp(rhs))
    } else {
        case_semisensitive_cmp(lhs.chars(), rhs.chars())
    }
}
