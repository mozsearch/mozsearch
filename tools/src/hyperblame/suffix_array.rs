//! A simple suffix array over `u32` symbols.
//!
//! The existing suffix array crates operate over `str` or `u8` which would
//! require us to encode our interned token identifiers into something like
//! UTF-8 and then deal with matches that could start in the middle of an
//! encoded token.  Since we just want to find the longest (token-wise) match
//! for a sequence of added tokens among the removed tokens of a revision, it's
//! easier to just build the suffix array directly over the token identifiers.
//!
//! Construction uses prefix doubling with radix sorting, which is O(n log n).
//! This is not as fast as SA-IS, but it's simple and robust against the highly
//! repetitive inputs we expect from source code, where naive comparison-based
//! suffix sorting would degrade badly.
//!
//! The value 0 is conventionally used by callers as a sentinel to separate
//! independent runs of tokens.  As long as queries never contain the sentinel,
//! no match can span a sentinel.

pub struct SuffixArray<'a> {
    text: &'a [u32],
    sa: Vec<u32>,
}

/// Stable counting sort of `input` into `output` using `key(x)` which must be
/// in `0..num_keys`.
fn counting_sort(
    input: &[u32],
    output: &mut [u32],
    num_keys: usize,
    counts: &mut Vec<u32>,
    key: impl Fn(u32) -> usize,
) {
    counts.clear();
    counts.resize(num_keys + 1, 0);
    for &x in input {
        counts[key(x) + 1] += 1;
    }
    for i in 1..counts.len() {
        counts[i] += counts[i - 1];
    }
    for &x in input {
        let k = key(x);
        output[counts[k] as usize] = x;
        counts[k] += 1;
    }
}

impl<'a> SuffixArray<'a> {
    pub fn new(text: &'a [u32]) -> Self {
        SuffixArray {
            text,
            sa: build_suffix_array(text),
        }
    }

    pub fn text(&self) -> &'a [u32] {
        self.text
    }

    /// The text position of the suffix with the given lexicographic rank.
    pub fn suffix(&self, rank: usize) -> usize {
        self.sa[rank] as usize
    }

    pub fn len(&self) -> usize {
        self.sa.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sa.is_empty()
    }

    /// Incrementally narrow the suffix array to find all suffixes which have
    /// `query[..k]` as a prefix for increasing values of k, stopping at the
    /// first k for which there are no matching suffixes or once `max_len` is
    /// reached.
    ///
    /// Returns a vec where the element at index `k - 1` is the half-open range
    /// of suffix ranks `(lo, hi)` whose suffixes start with `query[..k]`.  The
    /// length of the returned vec is therefore the length of the longest match
    /// for a prefix of `query` anywhere in the text.  The ranges are nested, so
    /// shorter matches have wider ranges which are supersets of longer
    /// matches' ranges.
    pub fn match_ranges(&self, query: &[u32], max_len: usize) -> Vec<(usize, usize)> {
        let mut ranges = vec![];
        let text = self.text;
        let n = text.len();
        let (mut lo, mut hi) = (0, self.sa.len());
        // Symbol at `pos` or None if past the end.  None sorts before all
        // symbols which matches the suffix array order where a suffix that is a
        // prefix of another suffix sorts first.
        let sym_at = |pos: usize| if pos < n { Some(text[pos]) } else { None };
        for (k, &c) in query.iter().enumerate().take(max_len) {
            let range = &self.sa[lo..hi];
            let new_lo = lo + range.partition_point(|&s| sym_at(s as usize + k) < Some(c));
            let new_hi = lo + range.partition_point(|&s| sym_at(s as usize + k) <= Some(c));
            if new_lo >= new_hi {
                break;
            }
            lo = new_lo;
            hi = new_hi;
            ranges.push((lo, hi));
        }
        ranges
    }
}

fn build_suffix_array(text: &[u32]) -> Vec<u32> {
    let n = text.len();
    if n == 0 {
        return vec![];
    }
    assert!(n < u32::MAX as usize, "text too long for u32 suffix array");

    // ## Initial ranks: compress the alphabet so ranks are in 0..n.
    let mut alphabet = text.to_vec();
    alphabet.sort_unstable();
    alphabet.dedup();
    let mut rank: Vec<u32> = text
        .iter()
        .map(|c| alphabet.binary_search(c).unwrap() as u32)
        .collect();
    let mut num_ranks = alphabet.len();
    drop(alphabet);

    let mut sa: Vec<u32> = vec![0; n];
    let mut tmp: Vec<u32> = vec![0; n];
    let mut counts: Vec<u32> = vec![];
    let ids: Vec<u32> = (0..n as u32).collect();
    counting_sort(&ids, &mut sa, num_ranks, &mut counts, |i| {
        rank[i as usize] as usize
    });
    drop(ids);

    let mut new_rank: Vec<u32> = vec![0; n];
    let mut k = 1;
    // Our ranks represent the ordering of the first `k` symbols of each suffix
    // at the top of each loop iteration.
    loop {
        // Recompute ranks for the first `k` symbols from the current `sa` if
        // they're not already distinct.  (On the first iteration `rank` is
        // already correct.)
        if num_ranks == n {
            break;
        }

        // ## Order by the second key: rank[i + k], with suffixes where i + k
        // runs off the end of the text sorting first.
        let mut idx = 0;
        for i in (n - k.min(n))..n {
            tmp[idx] = i as u32;
            idx += 1;
        }
        for &s in sa.iter() {
            if s as usize >= k {
                tmp[idx] = s - k as u32;
                idx += 1;
            }
        }
        debug_assert_eq!(idx, n);

        // ## Stable sort by the first key: rank[i]
        counting_sort(&tmp, &mut sa, num_ranks, &mut counts, |i| {
            rank[i as usize] as usize
        });

        // ## Compute the new ranks for the first 2k symbols.
        let second = |i: usize| -> Option<u32> { if i + k < n { Some(rank[i + k]) } else { None } };
        new_rank[sa[0] as usize] = 0;
        let mut r = 0;
        for j in 1..n {
            let (a, b) = (sa[j - 1] as usize, sa[j] as usize);
            if rank[a] != rank[b] || second(a) != second(b) {
                r += 1;
            }
            new_rank[b] = r;
        }
        std::mem::swap(&mut rank, &mut new_rank);
        num_ranks = r as usize + 1;
        k *= 2;
    }

    sa
}

#[cfg(test)]
mod tests {
    use super::*;

    fn naive_suffix_array(text: &[u32]) -> Vec<u32> {
        let mut sa: Vec<u32> = (0..text.len() as u32).collect();
        sa.sort_by(|&a, &b| text[a as usize..].cmp(&text[b as usize..]));
        sa
    }

    /// Tiny deterministic PRNG so we don't need a dependency.
    struct XorShift(u64);
    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    #[test]
    fn test_matches_naive() {
        let fixed: Vec<Vec<u32>> = vec![
            vec![],
            vec![5],
            vec![1, 1, 1, 1, 1],
            vec![3, 1, 2, 3, 1, 2, 3, 1],
            vec![2, 1, 0, 2, 1, 0, 2, 1],
            vec![9, 8, 7, 6, 5, 4, 3, 2, 1],
        ];
        for text in fixed {
            let sa = SuffixArray::new(&text);
            assert_eq!(sa.sa, naive_suffix_array(&text), "text {:?}", text);
        }

        let mut rng = XorShift(0x2545F4914F6CDD1D);
        for round in 0..200 {
            let len = (rng.next() % 300) as usize;
            // Small alphabets create lots of repetition which is where bugs
            // tend to hide.
            let alphabet = 1 + (rng.next() % if round % 2 == 0 { 3 } else { 50 }) as u32;
            let text: Vec<u32> = (0..len)
                .map(|_| (rng.next() % alphabet as u64) as u32)
                .collect();
            let sa = SuffixArray::new(&text);
            assert_eq!(sa.sa, naive_suffix_array(&text), "text {:?}", text);
        }
    }

    #[test]
    fn test_match_ranges() {
        // Two runs separated by the 0 sentinel.
        let text = vec![1, 2, 3, 4, 0, 2, 3, 5, 0];
        let sa = SuffixArray::new(&text);

        let ranges = sa.match_ranges(&[2, 3, 4, 9], 10);
        // [2], [2, 3], [2, 3, 4] match; [2, 3, 4, 9] does not.
        assert_eq!(ranges.len(), 3);
        let positions = |(lo, hi): (usize, usize)| {
            let mut v: Vec<usize> = (lo..hi).map(|r| sa.suffix(r)).collect();
            v.sort();
            v
        };
        assert_eq!(positions(ranges[0]), vec![1, 5]);
        assert_eq!(positions(ranges[1]), vec![1, 5]);
        assert_eq!(positions(ranges[2]), vec![1]);

        // A match can't span the sentinel.
        let ranges = sa.match_ranges(&[3, 4, 2], 10);
        assert_eq!(ranges.len(), 2);

        // max_len is respected.
        let ranges = sa.match_ranges(&[1, 2, 3, 4], 2);
        assert_eq!(ranges.len(), 2);

        // No match at all.
        assert!(sa.match_ranges(&[7], 10).is_empty());
    }
}
