//! How recently, and how much, symbols' code changed, in bins of age: the
//! digests behind `/query/`'s recency facets.  crossref computes them from a
//! tree's history (see `hyperblame::recency`) and puts them in the symbols'
//! crossref and jumpref data.

use std::collections::BTreeMap;
use std::ops::Range;

use serde::{Deserialize, Serialize};

/// The (exclusive) upper bounds of the bins' ages, in weeks before the indexed
/// revision: under 1 week, 1-2 weeks, 2-4 weeks, 1-2 months, 2-3 months, 3-6
/// months, 6-12 months, 1-2 years, 2-4 years, and older.
pub const BIN_WEEKS: [i64; 9] = [1, 2, 4, 9, 13, 26, 52, 104, 208];
pub const BINS: usize = BIN_WEEKS.len() + 1;

/// The bins' ages, for descriptions.
pub const BIN_LABELS: [&str; BINS] = [
    "under 1 week",
    "1-2 weeks",
    "2-4 weeks",
    "1-2 months",
    "2-3 months",
    "3-6 months",
    "6-12 months",
    "1-2 years",
    "2-4 years",
    "over 4 years",
];

/// The values of the "Last changed" facet (see `Recency::last_changed`): their
/// keys, names, and descriptions, and the bins of the changes they're for.
pub const LAST_CHANGED: [(&str, &str, &str, Range<usize>); 6] = [
    ("week", "past week", "Last changed under 1 week ago", 0..1),
    ("month", "past month", "Last changed 1-4 weeks ago", 1..3),
    (
        "quarter",
        "past 3 months",
        "Last changed 1-3 months ago",
        3..5,
    ),
    ("year", "past year", "Last changed 3-12 months ago", 5..7),
    ("years", "1-4 years", "Last changed 1-4 years ago", 7..9),
    ("older", "4+ years", "Last changed over 4 years ago", 9..10),
];

/// The "Last changed" value of what has no digest (ex: generated files).
pub const LAST_CHANGED_UNKNOWN: (&str, &str, &str) =
    ("unknown", "unknown", "No history (ex: generated files)");

/// The number of tokens changed (not counting moves) in each bin of age,
/// newest first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recency(pub [u32; BINS]);

/// Where a symbol's digest is from (see `hyperblame::recency`): the history
/// context of its (first) definition, or declaration, and its file, and how
/// many definitions (or declarations) it sums.  Which says when the history has
/// no context of the symbol's own (ex: a method declared in a class, or whose
/// definition the tokenizer didn't make a context of), so the digest is its
/// class's or namespace's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecencyFrom {
    pub path: String,
    pub context: String,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub sites: u32,
}

fn one() -> u32 {
    1
}

fn is_one(n: &u32) -> bool {
    *n == 1
}

impl RecencyFrom {
    /// A description of whose history a digest is, for the symbol `pretty`
    /// (ex: "History of Foo::Bar, from history context ns in a.cpp").
    pub fn describe(&self, pretty: &str) -> String {
        let others = match self.sites {
            0 | 1 => String::new(),
            2 => " (and 1 more)".to_string(),
            n => format!(" (and {} more)", n - 1),
        };
        format!(
            "History of {}\nfrom history context {} in {}{}",
            pretty, self.context, self.path, others
        )
    }
}

/// A file's history digests, for what symbols' don't cover: the whole file's
/// (for file name matches), and those of the contexts that symbols don't have
/// digests for (for lines in them): the top level ("%") and namespaces (which
/// span files), by context, without the contexts nested in them.  Files
/// without analysis only have the whole file's (which their lines get, since
/// they don't have contexts).  crossref writes them to `file-recency`, keyed by
/// path.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecency {
    pub file: Recency,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub scopes: BTreeMap<String, Recency>,
}

impl FileRecency {
    /// The digest of a line in the file without its own, in a scope (its
    /// context's pretty name, or "%" for the top level): the scope's, by the
    /// name or its longest suffix (ex: "tests" for a Rust module whose
    /// context is "foo::tests", since modules' contexts are their own names),
    /// or the whole file's if the file has no scopes (no analysis), with the
    /// scope's name if it's a scope's.
    pub fn scope(&self, scope: &str) -> Option<(Recency, Option<&str>)> {
        if self.scopes.is_empty() {
            return Some((self.file, None));
        }
        let mut name = scope;
        loop {
            if let Some((key, recency)) = self.scopes.get_key_value(name) {
                return Some((*recency, Some(key.as_str())));
            }
            name = name.split_once("::")?.1;
        }
    }
}

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

    /// The key of the "Last changed" value (see `LAST_CHANGED`) of what has
    /// this digest, if any.
    pub fn last_changed(recency: Option<&Recency>) -> &'static str {
        recency
            .and_then(Recency::newest_bin)
            .and_then(|bin| {
                LAST_CHANGED
                    .iter()
                    .find(|(_, _, _, bins)| bins.contains(&bin))
            })
            .map_or(LAST_CHANGED_UNKNOWN.0, |(key, _, _, _)| key)
    }

    /// How much a bin changed, from 0 (nothing) to 5, roughly logarithmically,
    /// for shading.
    pub fn level(&self, bin: usize) -> u8 {
        match self.0[bin] {
            0 => 0,
            1..=9 => 1,
            10..=49 => 2,
            50..=199 => 3,
            200..=999 => 4,
            _ => 5,
        }
    }

    /// A description of the changes, ex: "Tokens changed: 150 under 1 week
    /// ago, 20 3-6 months ago".
    pub fn describe(&self) -> String {
        let changes: Vec<String> = self
            .0
            .iter()
            .zip(BIN_LABELS)
            .filter(|(tokens, _)| **tokens > 0)
            .map(|(tokens, label)| format!("{} {} ago", tokens, label))
            .collect();
        if changes.is_empty() {
            "No changes".to_string()
        } else {
            format!("Tokens changed: {}", changes.join(", "))
        }
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
        assert_eq!(Recency::last_changed(Some(&recency)), "quarter");
        assert_eq!(Recency::last_changed(None), "unknown");
        assert_eq!(recency.level(3), 1);
        assert_eq!(recency.level(0), 0);
        assert_eq!(
            recency.describe(),
            "Tokens changed: 6 1-2 months ago, 2 2-3 months ago"
        );
        let from = RecencyFrom {
            path: "a.cpp".to_string(),
            context: "ns".to_string(),
            sites: 3,
        };
        assert_eq!(
            from.describe("ns::Foo::Bar"),
            "History of ns::Foo::Bar\nfrom history context ns in a.cpp (and 2 more)"
        );
        assert_eq!(
            serde_json::to_string(&RecencyFrom { sites: 1, ..from }).unwrap(),
            r#"{"path":"a.cpp","context":"ns"}"#
        );
    }

    #[test]
    fn test_file_scopes() {
        let digest = |tokens: u32| Recency([tokens, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let file = FileRecency {
            file: digest(10),
            scopes: [
                ("%".to_string(), digest(1)),
                ("tests".to_string(), digest(2)),
            ]
            .into_iter()
            .collect(),
        };
        assert_eq!(file.scope("%"), Some((digest(1), Some("%"))));
        assert_eq!(file.scope("a::b::tests"), Some((digest(2), Some("tests"))));
        assert_eq!(file.scope("a::b"), None);
        let unanalyzed = FileRecency {
            file: digest(10),
            scopes: BTreeMap::new(),
        };
        assert_eq!(unanalyzed.scope("%"), Some((digest(10), None)));
    }
}
