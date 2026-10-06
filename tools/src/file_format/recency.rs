//! How recently, and how much, symbols' code changed, in bins of age: the
//! digests behind `/query/`'s recency facets.  crossref computes them from a
//! tree's history (see `hyperblame::recency`) and puts them in the symbols'
//! crossref and jumpref data.

use serde::{Deserialize, Serialize};

/// The (exclusive) upper bounds of the bins' ages, in weeks before the indexed
/// revision: under 1 week, 1-2 weeks, 2-4 weeks, 1-2 months, 2-3 months, 3-6
/// months, 6-12 months, 1-2 years, 2-4 years, and older.
pub const BIN_WEEKS: [i64; 9] = [1, 2, 4, 9, 13, 26, 52, 104, 208];
pub const BINS: usize = BIN_WEEKS.len() + 1;

/// The number of tokens changed (not counting moves) in each bin of age,
/// newest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recency(pub [u32; BINS]);

impl Recency {
    /// The bin of a change `days` days before the indexed revision.
    pub fn bin_for_age_days(days: i64) -> usize {
        BIN_WEEKS
            .iter()
            .position(|&weeks| days < weeks * 7)
            .unwrap_or(BINS - 1)
    }

    pub fn add(&mut self, bin: usize, tokens: u32) {
        self.0[bin] = self.0[bin].saturating_add(tokens);
    }

    pub fn accumulate(&mut self, other: &Recency) {
        for (bin, tokens) in other.0.iter().enumerate() {
            self.add(bin, *tokens);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|&tokens| tokens == 0)
    }

    /// The newest bin with changes, if any.
    pub fn newest_bin(&self) -> Option<usize> {
        self.0.iter().position(|&tokens| tokens > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bins() {
        assert_eq!(Recency::bin_for_age_days(0), 0);
        assert_eq!(Recency::bin_for_age_days(6), 0);
        assert_eq!(Recency::bin_for_age_days(7), 1);
        assert_eq!(Recency::bin_for_age_days(60), 3);
        assert_eq!(Recency::bin_for_age_days(400), 7);
        assert_eq!(Recency::bin_for_age_days(5000), 9);
        // (Changes "after" the indexed revision, ex: from clock skew, are new.)
        assert_eq!(Recency::bin_for_age_days(-3), 0);

        let mut recency = Recency::default();
        assert_eq!(recency.newest_bin(), None);
        recency.add(3, 5);
        recency.accumulate(&Recency([0, 0, 0, 1, 2, 0, 0, 0, 0, 0]));
        assert_eq!(recency.0[3], 6);
        assert_eq!(recency.newest_bin(), Some(3));
        assert_eq!(
            serde_json::to_string(&recency).unwrap(),
            "[0,0,0,6,2,0,0,0,0,0]"
        );
    }
}
