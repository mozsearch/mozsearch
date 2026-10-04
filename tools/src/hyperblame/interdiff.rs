//! Interdiffs: how one version of a patch (B) differs from another (A) in a
//! file, ex: a reland from the landing it relands.  Each side is a commit or a
//! stack of commits, and its patch's tokens are the ones its commits
//! introduced; commits on neither side (ex: what B was rebased onto) are the
//! base.
//!
//! A text interdiff (a diff of the two versions of the file, or of the two
//! diffs) mixes the patches' differences with what changed in between.  The
//! token-centric history tells them apart, since every token has an identity
//! (where it was introduced) which it keeps until it changes, and backouts
//! restore the identities of the tokens they put back:
//! - A token which both sides removed has the same identity on both sides.
//! - The tokens a side added have its commits' identities, so B's added tokens
//!   can't be A's (even in an unchanged reland), but they're the same as A's
//!   if they match A's added tokens in an alignment of the two versions of the
//!   file whose other tokens only match tokens with the same identity, which
//!   keeps the matches within the same places in the file.
//!
//! We present B's diff with its added and removed tokens marked as the same as
//! A's or new in B, the tokens A removed which B keeps, the tokens from
//! outside both patches, and A's added tokens which B doesn't have; see
//! `format::format_interdiff`.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use serde::Serialize;
use similar::DiffOp;

use super::inference::diff_token_lines;

/// A token's identity: the revision, path, and (1-based) index of the token in
/// the file where it was introduced.
pub type TokenId = (String, String, u32);

/// A token of a file in a revision.
#[derive(Clone, Debug)]
pub struct IdToken {
    pub id: TokenId,
    pub text: String,
    /// The pretty identifier of the token's structural context (ex: the
    /// method it's in), or "%" if it has none; see `TokenLine::context`.
    pub context: String,
    /// The token's byte range in the source.
    pub range: Range<usize>,
    /// The (0-based) source line the token is on.
    pub line: usize,
}

/// The identities of the tokens of `before` which `after` doesn't have.
pub fn removed(before: &[IdToken], after: &[IdToken]) -> HashSet<TokenId> {
    let after: HashSet<&TokenId> = after.iter().map(|t| &t.id).collect();
    before
        .iter()
        .filter(|t| !after.contains(&t.id))
        .map(|t| t.id.clone())
        .collect()
}

/// A side of an interdiff in a file.
pub struct Side<'a> {
    /// The side's commits.
    pub revs: &'a HashSet<String>,
    /// The file before the side's first commit, and after its last.
    pub base: &'a [IdToken],
    pub post: &'a [IdToken],
    /// The tokens of `base` which the side's commits removed (all of the ones
    /// `post` doesn't have, unless there are other commits in between).
    pub removed: &'a HashSet<TokenId>,
}

/// What an interdiff says about a token of B's diff.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum Mark {
    /// B added it, and A added the same token there.
    #[serde(rename = "same")]
    Same,
    /// B added it, and A didn't.
    #[serde(rename = "new")]
    New,
    /// A removed it, but B keeps it.
    #[serde(rename = "kept")]
    Kept,
    /// Neither side added it, but A's version of the file doesn't have it (ex:
    /// a commit B was rebased onto added it).
    #[serde(rename = "base")]
    Base,
    /// B removed it, and so did A.
    #[serde(rename = "rm-same")]
    RemovedSame,
    /// B removed it, and A didn't.
    #[serde(rename = "rm-new")]
    RemovedNew,
    /// A added it, and B didn't (one of A's tokens).
    #[serde(rename = "only-a")]
    OnlyA,
}

impl Mark {
    /// Whether the token is a difference between the patches (rather than
    /// one of the patches' common changes, or from neither patch).
    pub fn differs(self) -> bool {
        matches!(
            self,
            Mark::New | Mark::Kept | Mark::RemovedNew | Mark::OnlyA
        )
    }

    pub fn class(self) -> &'static str {
        match self {
            Mark::Same => "same",
            Mark::New => "new",
            Mark::Kept => "kept",
            Mark::Base => "base",
            Mark::RemovedSame => "rm-same",
            Mark::RemovedNew => "rm-new",
            Mark::OnlyA => "only-a",
        }
    }
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct Counts {
    pub same: usize,
    pub new: usize,
    #[serde(rename = "onlyA")]
    pub only_a: usize,
    pub kept: usize,
    pub base: usize,
    #[serde(rename = "removedSame")]
    pub removed_same: usize,
    #[serde(rename = "removedNew")]
    pub removed_new: usize,
}

impl std::ops::AddAssign<&Counts> for Counts {
    fn add_assign(&mut self, other: &Counts) {
        self.same += other.same;
        self.new += other.new;
        self.only_a += other.only_a;
        self.kept += other.kept;
        self.base += other.base;
        self.removed_same += other.removed_same;
        self.removed_new += other.removed_new;
    }
}

#[derive(Debug)]
pub struct Interdiff {
    /// The marks of the tokens of B's post, by index.
    pub post_marks: Vec<Option<Mark>>,
    /// The marks of the tokens of B's base, by index.
    pub base_marks: Vec<Option<Mark>>,
    /// The tokens of A's post which only A added, by index, with the (0-based)
    /// line of B's post that the nearest matched token before them is on (None
    /// if there isn't one), which is where B would have them.
    pub only_a: Vec<(usize, Option<usize>)>,
    pub counts: Counts,
}

/// The interdiff of `b` from `a`; see the module docs.
pub fn interdiff(a: &Side, b: &Side) -> Interdiff {
    let a_post_ids: HashSet<&TokenId> = a.post.iter().map(|t| &t.id).collect();
    let b_post_ids: HashSet<&TokenId> = b.post.iter().map(|t| &t.id).collect();
    // A side's added tokens which the other side doesn't have match tokens
    // with the same text, and other tokens match the tokens with the same
    // identity.
    let key = |t: &IdToken, side: &Side, other_ids: &HashSet<&TokenId>| {
        if side.revs.contains(&t.id.0) && !other_ids.contains(&t.id) {
            format!("t\0{}", t.text)
        } else {
            format!("i\0{}\0{}\0{}", t.id.0, t.id.1, t.id.2)
        }
    };
    let a_keys: Vec<String> = a.post.iter().map(|t| key(t, a, &b_post_ids)).collect();
    let b_keys: Vec<String> = b.post.iter().map(|t| key(t, b, &a_post_ids)).collect();
    let a_refs: Vec<&str> = a_keys.iter().map(String::as_str).collect();
    let b_refs: Vec<&str> = b_keys.iter().map(String::as_str).collect();
    // The B post token matched to each A post token, and vice versa.
    let mut a_to_b: HashMap<usize, usize> = HashMap::new();
    let mut b_matched = vec![false; b.post.len()];
    for op in diff_token_lines(&a_refs, &b_refs) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            for k in 0..len {
                a_to_b.insert(old_index + k, new_index + k);
                b_matched[new_index + k] = true;
            }
        }
    }

    let mut counts = Counts::default();
    let post_marks = b
        .post
        .iter()
        .enumerate()
        .map(|(j, t)| {
            let mark = if b.revs.contains(&t.id.0) {
                if b_matched[j] && !a_post_ids.contains(&t.id) {
                    Some(Mark::Same)
                } else if a_post_ids.contains(&t.id) {
                    // (A has it too, so B's base had it: B descends from A's
                    // side, or the sides share commits.)
                    None
                } else {
                    Some(Mark::New)
                }
            } else if a.removed.contains(&t.id) {
                Some(Mark::Kept)
            } else if !a_post_ids.contains(&t.id) {
                Some(Mark::Base)
            } else {
                None
            };
            match mark {
                Some(Mark::Same) => counts.same += 1,
                Some(Mark::New) => counts.new += 1,
                Some(Mark::Kept) => counts.kept += 1,
                Some(Mark::Base) => counts.base += 1,
                _ => {}
            }
            mark
        })
        .collect();
    let base_marks = b
        .base
        .iter()
        .map(|t| {
            if !b.removed.contains(&t.id) {
                None
            } else if a.removed.contains(&t.id) {
                counts.removed_same += 1;
                Some(Mark::RemovedSame)
            } else {
                counts.removed_new += 1;
                Some(Mark::RemovedNew)
            }
        })
        .collect();

    let mut only_a = vec![];
    let mut anchor = None;
    for (i, t) in a.post.iter().enumerate() {
        if let Some(&j) = a_to_b.get(&i) {
            anchor = Some(b.post[j].line);
        } else if a.revs.contains(&t.id.0) && !b_post_ids.contains(&t.id) {
            only_a.push((i, anchor));
        }
    }
    counts.only_a = only_a.len();

    Interdiff {
        post_marks,
        base_marks,
        only_a,
        counts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tokens from "rev:lineno:text" words on lines separated by "|".
    fn tokens(spec: &str) -> Vec<IdToken> {
        let mut out = vec![];
        let mut offset = 0;
        for (line, words) in spec.split('|').enumerate() {
            for word in words.split_whitespace() {
                let mut parts = word.splitn(3, ':');
                let rev = parts.next().unwrap();
                let lineno = parts.next().unwrap().parse().unwrap();
                let text = parts.next().unwrap();
                out.push(IdToken {
                    id: (rev.to_string(), "f".to_string(), lineno),
                    text: text.to_string(),
                    context: "%".to_string(),
                    range: offset..offset + text.len(),
                    line,
                });
                offset += text.len() + 1;
            }
        }
        out
    }

    fn revs(revs: &[&str]) -> HashSet<String> {
        revs.iter().map(|r| r.to_string()).collect()
    }

    fn marks(interdiff: &Interdiff, tokens: &[IdToken], base: bool) -> Vec<String> {
        let marks = if base {
            &interdiff.base_marks
        } else {
            &interdiff.post_marks
        };
        tokens
            .iter()
            .zip(marks)
            .filter_map(|(t, m)| m.map(|m| format!("{}={}", t.text, m.class())))
            .collect()
    }

    #[test]
    fn test_reland_with_changes() {
        // A (a) replaced `return;` with `return rv; assert(x);`, and was
        // backed out; B (b) relanded it as `return rv; log(rv);` after a base
        // commit (c) added `init();` at the top.
        let a_base = tokens("o:1:if o:2:{ | o:3:return o:4:; | o:5:}");
        let a_post =
            tokens("o:1:if o:2:{ | o:3:return a:1:rv o:4:; | a:2:assert a:3:x a:4:; | o:5:}");
        let b_base = tokens("c:1:init c:2:; | o:1:if o:2:{ | o:3:return o:4:; | o:5:}");
        let b_post = tokens(
            "c:1:init c:2:; | o:1:if o:2:{ | o:3:return b:1:rv o:4:; | b:2:log b:3:rv b:4:; | o:5:}",
        );
        let (a_revs, b_revs) = (revs(&["a"]), revs(&["b"]));
        let (a_removed, b_removed) = (removed(&a_base, &a_post), removed(&b_base, &b_post));
        let a = Side {
            revs: &a_revs,
            base: &a_base,
            post: &a_post,
            removed: &a_removed,
        };
        let b = Side {
            revs: &b_revs,
            base: &b_base,
            post: &b_post,
            removed: &b_removed,
        };
        let result = interdiff(&a, &b);
        assert_eq!(
            marks(&result, &b_post, false),
            vec![
                "init=base",
                ";=base",
                "rv=same",
                "log=new",
                "rv=new",
                ";=same"
            ]
        );
        let only_a: Vec<_> = result
            .only_a
            .iter()
            .map(|&(i, anchor)| (a_post[i].text.as_str(), anchor))
            .collect();
        // They go after B's line 2 (`return rv;`): the `;` of `assert(x);`
        // matched `log(rv);`'s.
        assert_eq!(only_a, vec![("assert", Some(2)), ("x", Some(2))]);
        assert_eq!(
            result.counts,
            Counts {
                same: 2,
                new: 2,
                only_a: 2,
                kept: 0,
                base: 2,
                removed_same: 0,
                removed_new: 0,
            }
        );
    }

    #[test]
    fn test_removals() {
        // A removed `b` and `c`; B removed `c` and `d`, keeping `b`.
        let base = tokens("o:1:a o:2:b | o:3:c | o:4:d o:5:e");
        let a_post = tokens("o:1:a | o:4:d o:5:e");
        let b_post = tokens("o:1:a o:2:b | o:5:e");
        let (a_revs, b_revs) = (revs(&["a"]), revs(&["b"]));
        let (a_removed, b_removed) = (removed(&base, &a_post), removed(&base, &b_post));
        let a = Side {
            revs: &a_revs,
            base: &base,
            post: &a_post,
            removed: &a_removed,
        };
        let b = Side {
            revs: &b_revs,
            base: &base,
            post: &b_post,
            removed: &b_removed,
        };
        let result = interdiff(&a, &b);
        assert_eq!(marks(&result, &b_post, false), vec!["b=kept"]);
        assert_eq!(marks(&result, &base, true), vec!["c=rm-same", "d=rm-new"]);
        assert!(result.only_a.is_empty());
    }

    #[test]
    fn test_descendant() {
        // B is a follow-up to A, which it keeps: A's tokens are B's base.
        let a_base = tokens("o:1:x");
        let a_post = tokens("o:1:x a:1:y");
        let b_post = tokens("o:1:x a:1:y b:1:z");
        let (a_revs, b_revs) = (revs(&["a"]), revs(&["b"]));
        let (a_removed, b_removed) = (removed(&a_base, &a_post), removed(&a_post, &b_post));
        let a = Side {
            revs: &a_revs,
            base: &a_base,
            post: &a_post,
            removed: &a_removed,
        };
        let b = Side {
            revs: &b_revs,
            base: &a_post,
            post: &b_post,
            removed: &b_removed,
        };
        let result = interdiff(&a, &b);
        assert_eq!(marks(&result, &b_post, false), vec!["z=new"]);
        assert!(result.only_a.is_empty());
    }
}
