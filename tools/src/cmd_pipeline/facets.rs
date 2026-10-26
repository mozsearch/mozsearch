//! Faceting of results, which groups them hierarchically by some aspect of
//! them (ex: the directories of their paths) when there are usefully sized
//! groups; see `ResultFacetRoot`.  Used by the "compile-results" pipeline
//! command for `/query/` results, and by the `/explore/` pages and interdiff
//! summaries (see `format::explore_facets`), whose files `file_facets`
//! facets for facet_bar.liquid and facets.js, as for `/query/`'s results.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;
use serde_json::json;
use ustr::{Ustr, ustr};

use super::interface::{ResultFacetGroup, ResultFacetKind, ResultFacetRoot};
use super::recency_html::sparkline;
use crate::file_format::recency::{LAST_CHANGED, LAST_CHANGED_UNKNOWN, Recency};
use crate::tree_sitter_support::preprocessor::{is_c_family, joined_condition};

/// Faceting support logic; the ResultFacetKind bakes in rules.
pub struct MaybeFacetRoot {
    pub kind: ResultFacetKind,
    pub root: MaybeFacetGroup,
}

impl MaybeFacetRoot {
    pub fn new(kind: ResultFacetKind) -> MaybeFacetRoot {
        MaybeFacetRoot {
            kind,
            root: MaybeFacetGroup::default(),
        }
    }

    /// Place the value within a fully built-out hierarchy.  We don't do dynamic
    /// hierarchy creation as things collide; instead we just create it all and
    /// then collapse it out of existence during the `compile` phase.
    pub fn place_item(&mut self, mut pieces: Vec<Ustr>, value: Ustr) {
        pieces.reverse();
        self.root.place_item(pieces, value);
    }

    /// Determine whether there's enough variety that faceting is appropriate,
    /// and if so, return a fully populated `ResultFacetRoot` according to the
    /// rules for this root's `ResultFacetKind`.
    ///
    /// The general algorithm here is:
    /// - Determine if each `MaybeFacetGroup` is "sole" (just one group),
    ///   "clumped" (has multiple sub-groups that meet the clump threshold),
    ///   "clump-able" (has one sub-group that meets the clump threshold and the
    ///   other groups can be clumped into an "Other" catch-all, if allowed)
    ///   or "sparse" (has sub-groups that don't meet the clump threshold).
    /// - A "sole" group that has a "sole" child gets merged with the child.
    ///   This happens for path-based faceting where multiple directory segments
    ///   may be shared in common with no deviation.
    /// -
    pub fn compile(self) -> Option<ResultFacetRoot> {
        if self.root.count == 0 {
            return None;
        }

        let (label, clump_thresh, other) = match self.kind {
            ResultFacetKind::SymbolByRelation => ("Relation".to_string(), 0, None),
            ResultFacetKind::PathByPath => ("Path".to_string(), 3, Some("*".to_string())),
            ResultFacetKind::PathByKind => ("Path kind".to_string(), 0, None),
            ResultFacetKind::PathBySubsystem => ("Subsystem".to_string(), 0, None),
        };
        let (compiled, breadth) = self.root.compile("".to_string(), clump_thresh, other);
        if breadth > 1 {
            Some(ResultFacetRoot {
                label,
                kind: self.kind,
                groups: compiled.nested_groups,
            })
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct MaybeFacetGroup {
    pub nested_groups: BTreeMap<Ustr, MaybeFacetGroup>,
    pub values: Vec<Ustr>,
    /// Count of the values stored in this group in `values` and any nested
    /// groups.  This value will always be at least 1.
    pub count: u32,
}

impl MaybeFacetGroup {
    pub fn place_item(&mut self, mut reversed_pieces: Vec<Ustr>, value: Ustr) {
        self.count += 1;
        if let Some(next_piece) = reversed_pieces.pop() {
            self.nested_groups
                .entry(next_piece)
                .or_default()
                .place_item(reversed_pieces, value);
        } else {
            self.values.push(value);
        }
    }

    pub fn flatten(mut self) -> Vec<Ustr> {
        for subgroup in self.nested_groups.into_values() {
            let mut sub_flattened = subgroup.flatten();
            self.values.append(&mut sub_flattened);
        }

        self.values
    }

    /// Compiles the current group, returning the compiled result and the
    /// maximum number of nested groups known in the returned sub-tree which
    /// we're going to call the breadth.
    pub fn compile(
        mut self,
        prefix: String,
        clump_thresh: u32,
        other: Option<String>,
    ) -> (ResultFacetGroup, u32) {
        if self.nested_groups.is_empty() {
            // No sub-groups means we are a leaf node and should return as-is.
            return (
                ResultFacetGroup {
                    label: prefix,
                    values: self.values,
                    nested_groups: vec![],
                    count: self.count,
                },
                1,
            );
        } else if self.nested_groups.len() == 1 {
            let (sole_name, sole_group) = self.nested_groups.into_iter().next().unwrap();
            let (mut sole_compiled, breadth) = sole_group.compile(
                prefix.clone() + sole_name.as_str(),
                clump_thresh,
                other.clone(),
            );

            if self.values.is_empty() {
                // Collapse us into the nested group
                return (sole_compiled, breadth);
            }

            // We have values of our own and so we either want to fold the nested
            // group's contents into our own or retain our group and it as a
            // nested group.
            if breadth > 1 {
                // There's a tree somewhere down there, so just nest.
                return (
                    ResultFacetGroup {
                        label: prefix,
                        values: self.values,
                        nested_groups: vec![sole_compiled],
                        count: self.count,
                    },
                    breadth,
                );
            } else {
                // There's no tree below us, so fold its contents into us.  Note
                // that inductively according to this heuristic, we know the
                // sole_compiled will have no nested_groups and instead only
                // values.
                self.values.append(&mut sole_compiled.values);
                return (
                    ResultFacetGroup {
                        label: prefix,
                        values: self.values,
                        nested_groups: vec![],
                        count: self.count,
                    },
                    1,
                );
            }
        }

        // So there must be multiple nested_groups; the question is now how many
        // meet our clump criteria.
        let mut clump_hit_count: u32 = 0;
        let mut clump_miss_count: u32 = 0;

        for group in self.nested_groups.values() {
            if group.count >= clump_thresh {
                clump_hit_count += 1;
            } else {
                clump_miss_count += 1;
            }
        }

        if clump_hit_count >= 2 || (clump_hit_count >= 1 && other.is_some()) {
            let mut nested_groups = vec![];
            let mut breadth: u32;

            // Yes, we're going to materialize this group and some sub-groups.
            if let Some(other_label) = other.as_ref().filter(|_| clump_miss_count > 0) {
                let mut other_group = ResultFacetGroup {
                    label: prefix.clone() + other_label.as_str(),
                    values: vec![],
                    nested_groups: vec![],
                    count: 0,
                };
                breadth = clump_hit_count + 1;

                for (name, group) in self.nested_groups {
                    if group.count >= clump_thresh {
                        let (sub_compiled, sub_breadth) = group.compile(
                            prefix.clone() + name.as_str(),
                            clump_thresh,
                            other.clone(),
                        );
                        nested_groups.push(sub_compiled);
                        if sub_breadth > breadth {
                            breadth = sub_breadth;
                        }
                    } else {
                        other_group.count += group.count;
                        let mut sub_flattened = group.flatten();
                        other_group.values.append(&mut sub_flattened);
                    }
                }
                nested_groups.push(other_group);
            } else {
                breadth = self.nested_groups.len() as u32;
                // We don't need to worry about building up an "other" group.
                for (name, group) in self.nested_groups {
                    let (sub_compiled, sub_breadth) =
                        group.compile(prefix.clone() + name.as_str(), clump_thresh, other.clone());
                    nested_groups.push(sub_compiled);
                    if sub_breadth > breadth {
                        breadth = sub_breadth;
                    }
                }
            }

            (
                ResultFacetGroup {
                    label: prefix,
                    values: self.values,
                    nested_groups,
                    count: self.count,
                },
                breadth,
            )
        } else {
            // We're not going to materialize this group; just fold everything
            // in to ourselves.
            for subgroup in self.nested_groups.into_values() {
                let mut sub_flattened = subgroup.flatten();
                self.values.append(&mut sub_flattened);
            }

            (
                ResultFacetGroup {
                    label: prefix,
                    values: self.values,
                    nested_groups: vec![],
                    count: self.count,
                },
                1,
            )
        }
    }
}

/// A facet of a list of files (see `file_facets`), for facet_bar.liquid: its
/// key (as facets.js and URLs name it), label, and values, whether any of them
/// have nested values, and how many of them are elided (see `FacetView::new`).
#[derive(Serialize)]
pub struct FacetView {
    pub key: &'static str,
    pub label: String,
    pub values: Vec<FacetValueView>,
    pub nested: bool,
    pub more: u32,
}

/// How many of a facet's values show at first, and at most at each level of
/// a hierarchical facet's (counting the button showing the level's others).
const FACET_SHOWN: usize = 10;

impl FacetView {
    /// The facet, with which of its values show at first: at each level, if
    /// it has more than `FACET_SHOWN`, the biggest (by count) but one, in
    /// their order, with the others elided (behind a button showing them);
    /// and the nested values of the biggest values, biggest first, as long as
    /// the values showing stay within `FACET_SHOWN` (with the biggest of a
    /// value's nested values, if they don't all fit, but at least two; and
    /// not those of smaller values if the biggest's don't fit), since a facet
    /// is for narrowing results down a step at a time, and a long list of
    /// values, nested in others, isn't.
    pub fn new(key: &'static str, label: String, values: Vec<FacetValueView>) -> FacetView {
        FacetView::with_expanded(key, label, values, &[])
    }

    /// `FacetView::new`, but with the nested values of the values named by
    /// `expanded` (with or without their final "/", ex: "Core" for "Core/")
    /// showing at first whatever their number, and those values and their
    /// ancestors too (see `FacetsConfig::expanded_subsystems`).
    pub fn with_expanded(
        key: &'static str,
        label: String,
        mut values: Vec<FacetValueView>,
        expanded: &[String],
    ) -> FacetView {
        fn elide_all(values: &mut [FacetValueView]) {
            for value in values.iter_mut() {
                value.more = elide(&mut value.values, FACET_SHOWN).1;
                elide_all(&mut value.values);
            }
        }
        let (mut showing, more) = elide(&mut values, FACET_SHOWN);
        elide_all(&mut values);

        // The values to expand whatever their number, and their ancestors,
        // whose nested values count as showing.  Returns whether any are in
        // `values`.
        fn expand_named(
            values: &mut [FacetValueView],
            expanded: &[String],
            showing: &mut usize,
        ) -> bool {
            let mut any = false;
            for value in values.iter_mut() {
                let named = expanded
                    .iter()
                    .any(|name| name.trim_end_matches('/') == value.value.trim_end_matches('/'));
                let below = expand_named(&mut value.values, expanded, showing);
                if (named || below) && !value.values.is_empty() {
                    value.elided = false;
                    value.expanded = true;
                    let elided = value.values.iter().filter(|nested| nested.elided).count();
                    *showing += value.values.len() - elided + usize::from(elided > 0);
                    any = true;
                }
            }
            any
        }
        expand_named(&mut values, expanded, &mut showing);

        // The values showing with nested values which aren't expanded yet (in
        // those which are), as their indices at each level, which are expanded
        // biggest first if theirs fit.
        fn candidates(values: &[FacetValueView], at: &[usize], into: &mut Vec<Vec<usize>>) {
            for (i, value) in values.iter().enumerate() {
                if value.elided || value.values.is_empty() {
                    continue;
                }
                let at = [at, &[i]].concat();
                if value.expanded {
                    candidates(&value.values, &at, into);
                } else {
                    into.push(at);
                }
            }
        }
        fn node<'v>(values: &'v mut [FacetValueView], at: &[usize]) -> &'v mut FacetValueView {
            let (first, rest) = at.split_first().unwrap();
            rest.iter()
                .fold(&mut values[*first], |value, i| &mut value.values[*i])
        }
        let mut pending = vec![];
        candidates(&values, &[], &mut pending);
        while !pending.is_empty() {
            let biggest = (0..pending.len())
                .max_by_key(|i| {
                    let count = node(&mut values, &pending[*i]).count;
                    (count, std::cmp::Reverse(*i))
                })
                .unwrap();
            let at = pending.remove(biggest);
            let room = FACET_SHOWN.saturating_sub(showing);
            let value = node(&mut values, &at);
            if room < value.values.len().min(3) {
                break;
            }
            for nested in value.values.iter_mut() {
                nested.elided = false;
            }
            let (shown, more) = elide(&mut value.values, room);
            value.more = more;
            value.expanded = true;
            showing += shown;
            let mut nested = vec![];
            candidates(&value.values, &at, &mut nested);
            pending.extend(nested);
        }
        let nested = values.iter().any(|value| !value.values.is_empty());
        FacetView {
            key,
            label,
            values,
            nested,
            more,
        }
    }
}

/// Elide all but `limit` of `values`, if there are more: the biggest ones but
/// one (in their order), and a button for the others.  Returns how many show
/// (counting the button) and how many are elided.
fn elide(values: &mut [FacetValueView], limit: usize) -> (usize, u32) {
    if values.len() <= limit {
        return (values.len(), 0);
    }
    let mut by_count: Vec<usize> = (0..values.len()).collect();
    by_count.sort_by_key(|i| std::cmp::Reverse(values[*i].count));
    let shown = limit.saturating_sub(1).max(1);
    for i in &by_count[shown..] {
        values[*i].elided = true;
    }
    (shown + 1, (values.len() - shown) as u32)
}

/// A value of a facet (see `ResultFacetGroup`): its identity (its group's
/// label, ex: "dom/base/"), its name relative to its parent's (ex: "base/") and
/// a description, how many files it has, and its nested values.  And whether
/// it shows at first (see `FacetView::new`), and its nested values, and how
/// many of those are elided.
#[derive(Default, Serialize)]
pub struct FacetValueView {
    pub value: String,
    pub name: String,
    pub title: String,
    pub count: u32,
    pub values: Vec<FacetValueView>,
    /// A sparkline of the value's history digests (see `last_changed_facet`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sparkline: Option<String>,
    pub elided: bool,
    pub expanded: bool,
    pub more: u32,
}

/// A file to facet (see `file_facets`): its path, its path kind and whether
/// that's the indexed revision's (rather than guessed from its path), and its
/// subsystem (ex: "Firefox/Sidebar").
pub struct FacetFile {
    pub path: String,
    pub kind: Ustr,
    pub known: bool,
    pub subsystem: Option<Ustr>,
}

/// facets.js's data for a file (see facet_bar.liquid), as JSON: the values of
/// each facet it's in (with their ancestors), and its group (a sort key and a
/// name) for each way of grouping files.  And a title for its link (ex: its
/// subsystem).
#[derive(Default, Serialize)]
pub struct FileFacetData {
    pub facets: String,
    pub groups: String,
    pub title: String,
}

/// A tree's path kinds' keys and names (see per-file-info.toml), in their
/// display order.
pub struct PathKinds(pub Vec<(Ustr, Ustr)>);

impl PathKinds {
    /// The kind's place in the display order (after the tree's kinds if it's
    /// not one of them).
    pub fn order(&self, kind: &str) -> usize {
        self.0
            .iter()
            .position(|(key, _)| *key == kind)
            .unwrap_or(self.0.len())
    }

    pub fn name(&self, kind: &str) -> String {
        self.0
            .iter()
            .find(|(key, _)| *key == kind)
            .map_or_else(|| "Files".to_string(), |(_, name)| name.to_string())
    }
}

/// The subsystem facet's value for files without subsystems.
pub const UNKNOWN_SUBSYSTEM: &str = "?";

/// The "Last changed" facet of results' lines, by their history digests (see
/// `file_format::recency`), if any have them: its values (newest first, then
/// "unknown"), with how many files have lines in each and a sparkline of their
/// lines' digests, and the values of each file's lines, by path.
pub fn last_changed_facet<'a>(
    lines: impl IntoIterator<Item = (&'a str, Option<&'a Recency>)>,
) -> Option<(FacetView, HashMap<String, Vec<String>>)> {
    let mut any = false;
    let mut by_value: HashMap<&'static str, (HashSet<&str>, Recency)> = HashMap::new();
    let mut file_values: HashMap<String, BTreeSet<&'static str>> = HashMap::new();
    for (path, recency) in lines {
        any |= recency.is_some();
        let value = Recency::last_changed(recency);
        let (paths, total) = by_value.entry(value).or_default();
        paths.insert(path);
        if let Some(recency) = recency {
            total.accumulate(recency);
        }
        file_values
            .entry(path.to_string())
            .or_default()
            .insert(value);
    }
    if !any {
        return None;
    }
    let values = LAST_CHANGED
        .iter()
        .map(|(key, name, title, _)| (*key, *name, *title))
        .chain([LAST_CHANGED_UNKNOWN])
        .filter_map(|(key, name, title)| {
            let (paths, total) = by_value.get(key)?;
            Some(FacetValueView {
                value: key.to_string(),
                name: name.to_string(),
                title: title.to_string(),
                count: paths.len() as u32,
                values: vec![],
                sparkline: (!total.is_empty()).then(|| sparkline(total)),
                ..Default::default()
            })
        })
        .collect();
    let file_values = file_values
        .into_iter()
        .map(|(path, values)| (path, values.into_iter().map(String::from).collect()))
        .collect();
    Some((
        FacetView::new("recency", "Last changed".to_string(), values),
        file_values,
    ))
}

/// The "Preprocessor" facet's value for lines in no preprocessor conditionals
/// (or in languages without them).
pub const PP_NONE: &str = "";
/// The "Preprocessor" facet's value for lines whose conditionals aren't known:
/// C-family files' textual occurrences.
pub const PP_UNKNOWN: &str = "?";
/// How many conditions the "Preprocessor" facet shows at its top level, and
/// in each condition, before folding the rest into an "other" value.
const PP_SHOWN: [usize; 2] = [10, 5];

/// The "Preprocessor" facet of results' lines, by the preprocessor
/// conditionals' branches they're in (see `FlattenedLineSpan::pp`): for each
/// line, the conditions of its stack's prefixes (ex: "defined(XP_WIN)" and
/// "defined(XP_WIN)\ndefined(DEBUG)", the values in it), up to the most
/// common few at each level (see `PP_SHOWN`), and the rest's "other" value
/// (ex: "defined(XP_WIN)\n*"), or `PP_NONE` or `PP_UNKNOWN`.
pub struct PpFacet {
    pub view: FacetView,
    /// The values shown, which aren't folded into their levels' "other"s.
    shown: HashSet<String>,
}

impl PpFacet {
    /// The facet, if any of the lines (paths and stacks, with None for
    /// unknown) are in conditionals: its values (lines in none first, then
    /// the conditions with lines in the most files, and the other conditions,
    /// and lines in unknown ones), with how many files have lines in each, and
    /// the values of each file's lines, by path.
    pub fn new<'a>(
        lines: impl IntoIterator<Item = (&'a str, Option<&'a [String]>)>,
    ) -> Option<(PpFacet, HashMap<String, Vec<String>>)> {
        let lines: Vec<(&str, Option<&[String]>)> = lines.into_iter().collect();
        if !lines
            .iter()
            .any(|(_, pp)| pp.is_some_and(|pp| !pp.is_empty()))
        {
            return None;
        }
        // The files with lines in each value, and the values in each.
        let mut paths: HashMap<String, HashSet<&str>> = HashMap::new();
        let mut nested: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (path, pp) in &lines {
            let mut parent = String::new();
            for value in all_pp_values(path, *pp) {
                paths.entry(value.clone()).or_default().insert(path);
                nested.entry(parent).or_default().insert(value.clone());
                parent = value;
            }
        }
        let mut shown = HashSet::new();
        let values = pp_views(&String::new(), 0, &paths, &nested, &mut shown);
        let facet = PpFacet {
            view: FacetView::new("pp", "Preprocessor".to_string(), values),
            shown,
        };
        let mut file_values: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (path, pp) in &lines {
            file_values
                .entry(path.to_string())
                .or_default()
                .extend(facet.line_values(path, *pp));
        }
        let file_values = file_values
            .into_iter()
            .map(|(path, values)| (path, values.into_iter().collect()))
            .collect();
        Some((facet, file_values))
    }

    /// A line's values (see `PpFacet`).
    pub fn line_values(&self, path: &str, pp: Option<&[String]>) -> Vec<String> {
        let mut values = vec![];
        for value in all_pp_values(path, pp) {
            if self.shown.contains(&value) {
                values.push(value);
                continue;
            }
            let other = match value.rsplit_once('\n') {
                Some((parent, _)) => format!("{}\n*", parent),
                None => "*".to_string(),
            };
            values.push(other);
            break;
        }
        values
    }
}

/// A line's values of the "Preprocessor" facet without any folded into
/// "other"s (see `PpFacet`).
fn all_pp_values(path: &str, pp: Option<&[String]>) -> Vec<String> {
    match pp {
        None if is_c_family(path) => vec![PP_UNKNOWN.to_string()],
        None | Some([]) => vec![PP_NONE.to_string()],
        Some(stack) => (1..=stack.len()).map(|n| stack[..n].join("\n")).collect(),
    }
}

/// The views of the "Preprocessor" facet's values in `parent` (see
/// `PpFacet::new`), noting those shown.
fn pp_views(
    parent: &String,
    depth: usize,
    paths: &HashMap<String, HashSet<&str>>,
    nested: &BTreeMap<String, BTreeSet<String>>,
    shown: &mut HashSet<String>,
) -> Vec<FacetValueView> {
    let Some(values) = nested.get(parent) else {
        return vec![];
    };
    let count = |value: &String| paths[value].len() as u32;
    let mut conditions: Vec<&String> = values
        .iter()
        .filter(|value| *value != PP_NONE && *value != PP_UNKNOWN)
        .collect();
    conditions.sort_by_key(|value| std::cmp::Reverse(count(value)));
    let limit = PP_SHOWN[depth.min(PP_SHOWN.len() - 1)];
    // (Not an "other" of just one condition.)
    let folded = if conditions.len() > limit + 1 {
        conditions.split_off(limit)
    } else {
        vec![]
    };
    let condition = |value: &str| {
        let stack: Vec<String> = value.split('\n').map(str::to_string).collect();
        joined_condition(&stack)
    };
    let mut views = vec![];
    if values.contains(PP_NONE) {
        shown.insert(PP_NONE.to_string());
        views.push(FacetValueView {
            value: PP_NONE.to_string(),
            name: "Not conditional".to_string(),
            title: "Lines in no preprocessor conditionals (but include guards)".to_string(),
            count: count(&PP_NONE.to_string()),
            values: vec![],
            sparkline: None,
            ..Default::default()
        });
    }
    for value in conditions {
        shown.insert(value.clone());
        let name = value.rsplit('\n').next().unwrap_or(value);
        views.push(FacetValueView {
            value: value.clone(),
            name: truncated(name, 60),
            title: format!("#if {}", condition(value)),
            count: count(value),
            values: pp_views(value, depth + 1, paths, nested, shown),
            sparkline: None,
            ..Default::default()
        });
    }
    if !folded.is_empty() {
        let files: HashSet<&str> = folded
            .iter()
            .flat_map(|value| paths[*value].iter().copied())
            .collect();
        views.push(FacetValueView {
            value: if parent.is_empty() {
                "*".to_string()
            } else {
                format!("{}\n*", parent)
            },
            name: "other".to_string(),
            title: if parent.is_empty() {
                "The other preprocessor conditions".to_string()
            } else {
                format!("The other conditions in #if {}", condition(parent))
            },
            count: files.len() as u32,
            values: vec![],
            sparkline: None,
            ..Default::default()
        });
    }
    if values.contains(PP_UNKNOWN) {
        shown.insert(PP_UNKNOWN.to_string());
        views.push(FacetValueView {
            value: PP_UNKNOWN.to_string(),
            name: "Unknown".to_string(),
            title: "Textual occurrences in C-family files, whose conditionals aren't known"
                .to_string(),
            count: count(&PP_UNKNOWN.to_string()),
            values: vec![],
            sparkline: None,
            ..Default::default()
        });
    }
    views
}

/// How many places the "Placement" facet shows at its top level, and in each
/// place, before folding the rest into an "other" value.
const CTXTRA_SHOWN: [usize; 2] = [10, 8];

/// The "Placement" facet of results' lines, by where they are in their
/// contexts beyond their context symbols (see `FlattenedLineSpan::ctxtra`,
/// ex: a task's `fetches › toolchain`): for each line, its keys' prefixes
/// (ex: "fetches" and "fetches\ntoolchain", the values in it), up to the most
/// common few at each level, and the rest's "other" value (ex:
/// "fetches\n*"), or "" for lines without any.
pub struct CtxtraFacet {
    pub view: FacetView,
    shown: HashSet<String>,
}

impl CtxtraFacet {
    /// The facet, if any of the lines (paths and keys) have keys, and the
    /// values of each file's lines, by path.
    pub fn new<'a>(
        lines: impl IntoIterator<Item = (&'a str, &'a [String])>,
    ) -> Option<(CtxtraFacet, HashMap<String, Vec<String>>)> {
        let lines: Vec<(&str, &[String])> = lines.into_iter().collect();
        if !lines.iter().any(|(_, keys)| !keys.is_empty()) {
            return None;
        }
        let mut paths: HashMap<String, HashSet<&str>> = HashMap::new();
        let mut nested: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (path, keys) in &lines {
            let mut parent = String::new();
            for value in all_ctxtra_values(keys) {
                paths.entry(value.clone()).or_default().insert(path);
                nested.entry(parent).or_default().insert(value.clone());
                parent = value;
            }
        }
        let mut shown = HashSet::new();
        let values = ctxtra_views(&String::new(), 0, &paths, &nested, &mut shown);
        let facet = CtxtraFacet {
            view: FacetView::new("ctxtra", "Placement".to_string(), values),
            shown,
        };
        let mut file_values: HashMap<String, BTreeSet<String>> = HashMap::new();
        for (path, keys) in &lines {
            file_values
                .entry(path.to_string())
                .or_default()
                .extend(facet.line_values(keys));
        }
        let file_values = file_values
            .into_iter()
            .map(|(path, values)| (path, values.into_iter().collect()))
            .collect();
        Some((facet, file_values))
    }

    /// A line's values (see `CtxtraFacet`).
    pub fn line_values(&self, keys: &[String]) -> Vec<String> {
        let mut values = vec![];
        for value in all_ctxtra_values(keys) {
            if self.shown.contains(&value) {
                values.push(value);
                continue;
            }
            values.push(match value.rsplit_once('\n') {
                Some((parent, _)) => format!("{}\n*", parent),
                None => "*".to_string(),
            });
            break;
        }
        values
    }
}

fn all_ctxtra_values(keys: &[String]) -> Vec<String> {
    if keys.is_empty() {
        return vec![String::new()];
    }
    (1..=keys.len()).map(|n| keys[..n].join("\n")).collect()
}

fn ctxtra_views(
    parent: &String,
    depth: usize,
    paths: &HashMap<String, HashSet<&str>>,
    nested: &BTreeMap<String, BTreeSet<String>>,
    shown: &mut HashSet<String>,
) -> Vec<FacetValueView> {
    let Some(values) = nested.get(parent) else {
        return vec![];
    };
    let count = |value: &String| paths[value].len() as u32;
    let mut places: Vec<&String> = values.iter().filter(|value| !value.is_empty()).collect();
    places.sort_by_key(|value| std::cmp::Reverse(count(value)));
    let limit = CTXTRA_SHOWN[depth.min(CTXTRA_SHOWN.len() - 1)];
    let folded = if places.len() > limit + 1 {
        places.split_off(limit)
    } else {
        vec![]
    };
    let mut views = vec![];
    if values.contains("") {
        shown.insert(String::new());
        views.push(FacetValueView {
            value: String::new(),
            name: "None".to_string(),
            title: "Lines with no place in a context beyond its symbol (ex: textual occurrences)"
                .to_string(),
            count: count(&String::new()),
            values: vec![],
            sparkline: None,
            ..Default::default()
        });
    }
    for value in places {
        shown.insert(value.clone());
        let name = value.rsplit('\n').next().unwrap_or(value);
        views.push(FacetValueView {
            value: value.clone(),
            name: truncated(name, 60),
            title: format!("› {}", value.replace('\n', " › ")),
            count: count(value),
            values: ctxtra_views(value, depth + 1, paths, nested, shown),
            sparkline: None,
            ..Default::default()
        });
    }
    if !folded.is_empty() {
        let files: HashSet<&str> = folded
            .iter()
            .flat_map(|value| paths[*value].iter().copied())
            .collect();
        views.push(FacetValueView {
            value: if parent.is_empty() {
                "*".to_string()
            } else {
                format!("{}\n*", parent)
            },
            name: "other".to_string(),
            title: "The other places".to_string(),
            count: files.len() as u32,
            values: vec![],
            sparkline: None,
            ..Default::default()
        });
    }
    views
}

/// `text`, cut to at most `max` characters (with an ellipsis if cut).
fn truncated(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The facets of a list of files (ex: an `/explore/` page's, or `/query/`'s
/// results'), for facet_bar.liquid and facets.js: their path kinds (in the
/// tree's order), their subsystems (by product and then component, nested by
/// the components' "/"s, ex: "Core/DOM/Workers" in "Core/DOM/" in "Core/";
/// see per-file-info.toml), and their directories (the "Path" facet of
/// compile-results), the biggest first, and each file's data for facets.js, by
/// path.  `expanded_subsystems` are the subsystems whose nested values show at
/// first (see `FacetsConfig`).
pub fn file_facets(
    files: &[FacetFile],
    kinds: &PathKinds,
    expanded_subsystems: &[String],
) -> (Vec<FacetView>, HashMap<String, FileFacetData>) {
    let kind_order = |kind: &str| kinds.order(kind);
    let kind_name = |kind: &str| kinds.name(kind);

    // Each file's path kind, subsystem, and directory, placed in the facets.
    let mut kind_facet = MaybeFacetRoot::new(ResultFacetKind::PathByKind);
    let mut subsystem_facet = MaybeFacetRoot::new(ResultFacetKind::PathBySubsystem);
    let mut dir_facet = MaybeFacetRoot::new(ResultFacetKind::PathByPath);
    // (A subsystem which others are in, ex: "Core/Networking" and
    // "Core/Networking/HTTP", is its group's own files, like a directory's
    // files beside its subdirectories.)
    let groups: HashSet<&str> = files
        .iter()
        .filter_map(|file| file.subsystem.as_deref())
        .flat_map(|subsystem| {
            subsystem
                .match_indices('/')
                .map(move |(at, _)| &subsystem[..at])
        })
        .collect();
    for file in files {
        let path = ustr(&file.path);
        kind_facet.place_item(vec![file.kind], path);
        let subsystem_pieces = match file.subsystem.as_deref() {
            Some(subsystem) if groups.contains(subsystem) => format!("{}/", subsystem)
                .split_inclusive('/')
                .map(ustr)
                .collect(),
            Some(subsystem) => subsystem.split_inclusive('/').map(ustr).collect(),
            None => vec![ustr(UNKNOWN_SUBSYSTEM)],
        };
        subsystem_facet.place_item(subsystem_pieces, path);
        dir_facet.place_item(
            dir_of(&file.path).split_inclusive('/').map(ustr).collect(),
            path,
        );
    }

    // The facets' values, and the values each file is in (with their
    // ancestors), by facet.
    let mut memberships: HashMap<Ustr, BTreeMap<&'static str, Vec<String>>> = HashMap::new();
    // Products (ex: "Firefox"), their components and the groups of them (ex:
    // "Sidebar", "DOM"), and at the top, values of a product's components
    // alone named like Bugzilla does (ex: "Core :: DOM/Workers").
    let subsystem_name = |value: &str, parent: &str| {
        if value == UNKNOWN_SUBSYSTEM {
            return "Unknown".to_string();
        }
        let name = value.strip_prefix(parent).unwrap_or(value);
        let name = name.strip_suffix('/').unwrap_or(name);
        if parent.is_empty() {
            name.replacen('/', " :: ", 1)
        } else {
            name.to_string()
        }
    };
    // (The "*" groups of the other directories are "other", ex: "browser/
    // other" if the facet's top level has "browser/"'s.)
    let dir_name = |value: &str, parent: &str| {
        let name = value.strip_prefix(parent).unwrap_or(value);
        match name.strip_suffix('*') {
            Some("") => "other".to_string(),
            Some(dir) => format!("{} other", dir),
            None => name.to_string(),
        }
    };
    let mut facets = vec![];
    // (The directory facet is `/query/`'s "Path" facet, but "Directory" is
    // clearer next to the "Group by" choices.)
    type FacetSpec<'n> = (
        &'static str,
        Option<&'static str>,
        Option<ResultFacetRoot>,
        &'n dyn Fn(&str, &str) -> String,
    );
    let facet_specs: [FacetSpec; 3] = [
        ("kind", None, kind_facet.compile(), &|value, _| {
            kind_name(value)
        }),
        (
            "subsystem",
            None,
            subsystem_facet.compile(),
            &subsystem_name,
        ),
        ("dir", Some("Directory"), dir_facet.compile(), &dir_name),
    ];
    for (key, label, root, name) in facet_specs {
        let Some(root) = root else {
            continue;
        };
        fn view(
            key: &'static str,
            group: ResultFacetGroup,
            parent: &str,
            ancestors: &mut Vec<String>,
            name: &dyn Fn(&str, &str) -> String,
            memberships: &mut HashMap<Ustr, BTreeMap<&'static str, Vec<String>>>,
        ) -> FacetValueView {
            ancestors.push(group.label.clone());
            for value in &group.values {
                memberships
                    .entry(*value)
                    .or_default()
                    .entry(key)
                    .or_default()
                    .extend(ancestors.iter().cloned());
            }
            let values = group
                .nested_groups
                .into_iter()
                .map(|nested| view(key, nested, &group.label, ancestors, name, memberships))
                .collect();
            ancestors.pop();
            let title = match group.label.strip_suffix('*') {
                Some("") => "The other directories".to_string(),
                Some(dir) => format!("The other directories in {}", dir),
                None if group.label == UNKNOWN_SUBSYSTEM => {
                    "Files without subsystems, or not in the indexed revision".to_string()
                }
                None => group.label.clone(),
            };
            FacetValueView {
                name: name(&group.label, parent),
                value: group.label,
                title,
                count: group.count,
                values,
                sparkline: None,
                ..Default::default()
            }
        }
        let mut values: Vec<FacetValueView> = root
            .groups
            .into_iter()
            .map(|group| view(key, group, "", &mut vec![], name, &mut memberships))
            .collect();
        // (The unknown subsystem and the other directories last.)
        fn biggest_first(values: &mut [FacetValueView]) {
            values.sort_by(|a, b| {
                let last = |value: &FacetValueView| {
                    value.value == UNKNOWN_SUBSYSTEM || value.value.ends_with('*')
                };
                last(a)
                    .cmp(&last(b))
                    .then(b.count.cmp(&a.count))
                    .then_with(|| a.value.cmp(&b.value))
            });
            for value in values {
                biggest_first(&mut value.values);
            }
        }
        match key {
            "kind" => values.sort_by_key(|value| kind_order(&value.value)),
            _ => biggest_first(&mut values),
        }
        let expanded: &[String] = if key == "subsystem" {
            expanded_subsystems
        } else {
            &[]
        };
        facets.push(FacetView::with_expanded(
            key,
            label.map_or(root.label, str::to_string),
            values,
            expanded,
        ));
    }

    // Each file's data.
    let data = files
        .iter()
        .map(|file| {
            let subsystem_group = match file.subsystem {
                Some(subsystem) => {
                    let name = subsystem.replacen('/', " :: ", 1);
                    json!([name, name])
                }
                None => json!(["\u{10ffff}", "Unknown subsystem"]),
            };
            let dir_group = match dir_of(&file.path) {
                "" => json!(["", "(top level)"]),
                dir => json!([dir, dir]),
            };
            let groups = json!({
                "kind": [kind_order(&file.kind), kind_name(&file.kind)],
                "subsystem": subsystem_group,
                "dir": dir_group,
            });
            let title = match (file.known, file.subsystem) {
                (false, _) => "Not in the indexed revision, so its path kind is guessed from its path, and its subsystem is unknown".to_string(),
                (true, Some(subsystem)) => format!("Subsystem: {}", subsystem.replacen('/', " :: ", 1)),
                (true, None) => String::new(),
            };
            let facets = json!(memberships.remove(&ustr(&file.path)).unwrap_or_default());
            (
                file.path.clone(),
                FileFacetData {
                    facets: facets.to_string(),
                    groups: groups.to_string(),
                    title,
                },
            )
        })
        .collect();
    (facets, data)
}

/// The directory of the file at `path`, with its "/" (or "" at the top).
fn dir_of(path: &str) -> &str {
    match path.rfind('/') {
        Some(offset) => &path[..offset + 1],
        None => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ustr::ustr;

    #[test]
    fn test_pp_facet() {
        let stack = |conditions: &[&str]| -> Vec<String> {
            conditions.iter().map(|c| c.to_string()).collect()
        };
        let win_debug = stack(&["defined(XP_WIN)", "defined(DEBUG)"]);
        let win = stack(&["defined(XP_WIN)"]);
        let none = stack(&[]);
        let mut lines: Vec<(&str, Option<&[String]>)> = vec![
            ("a.cpp", Some(&win_debug)),
            ("a.cpp", Some(&none)),
            ("b.cpp", Some(&win)),
            // (A textual occurrence, and a JS line.)
            ("b.cpp", None),
            ("c.js", None),
        ];
        // (Lots of other conditions, with a file each, to fold.)
        let others: Vec<(String, Vec<String>)> = (0..12)
            .map(|i| (format!("o{}.h", i), vec![format!("C{}", i)]))
            .collect();
        lines.extend(
            others
                .iter()
                .map(|(path, stack)| (path.as_str(), Some(stack.as_slice()))),
        );
        let (facet, file_values) = PpFacet::new(lines.clone()).unwrap();
        fn names(values: &[FacetValueView]) -> Vec<String> {
            values
                .iter()
                .map(|value| {
                    let nested = names(&value.values);
                    if nested.is_empty() {
                        format!("{} {}", value.name, value.count)
                    } else {
                        format!("{} {} [{}]", value.name, value.count, nested.join(", "))
                    }
                })
                .collect()
        }
        assert_eq!(
            names(&facet.view.values),
            vec![
                "Not conditional 2",
                "defined(XP_WIN) 2 [defined(DEBUG) 1]",
                "C0 1",
                "C1 1",
                "C10 1",
                "C11 1",
                "C2 1",
                "C3 1",
                "C4 1",
                "C5 1",
                "C6 1",
                "other 3",
                "Unknown 1",
            ]
        );
        assert_eq!(
            facet.line_values("a.cpp", Some(&win_debug)),
            vec!["defined(XP_WIN)", "defined(XP_WIN)\ndefined(DEBUG)"]
        );
        assert_eq!(facet.line_values("o9.h", Some(&others[9].1)), vec!["*"]);
        assert_eq!(facet.line_values("b.cpp", None), vec![PP_UNKNOWN]);
        assert_eq!(facet.line_values("c.js", None), vec![PP_NONE]);
        let mut a_values = file_values["a.cpp"].clone();
        a_values.sort();
        assert_eq!(
            a_values,
            vec!["", "defined(XP_WIN)", "defined(XP_WIN)\ndefined(DEBUG)"]
        );

        // Without any conditionals, there's no facet.
        assert!(PpFacet::new([("a.cpp", Some(none.as_slice())), ("b.cpp", None)]).is_none());
    }

    #[test]
    fn test_path_facet_other() {
        // Directories with fewer than 3 items are lumped into "a/*".
        let mut facet = MaybeFacetRoot::new(ResultFacetKind::PathByPath);
        for (pieces, value) in [
            (vec!["a/", "b/"], "a/b/1"),
            (vec!["a/", "b/"], "a/b/2"),
            (vec!["a/", "b/"], "a/b/3"),
            (vec!["a/", "c/"], "a/c/1"),
            (vec!["a/", "d/"], "a/d/1"),
        ] {
            facet.place_item(pieces.into_iter().map(ustr).collect(), ustr(value));
        }
        let root = facet.compile().unwrap();
        let groups: Vec<(&str, u32)> = root
            .groups
            .iter()
            .map(|group| (group.label.as_str(), group.count))
            .collect();
        assert_eq!(groups, vec![("a/b/", 3), ("a/*", 2)]);
        assert_eq!(root.groups[1].values, vec![ustr("a/c/1"), ustr("a/d/1")]);
    }

    #[test]
    fn test_subsystem_nesting() {
        let file = |path: &str, subsystem: Option<&str>| FacetFile {
            path: path.to_string(),
            kind: ustr("normal"),
            known: true,
            subsystem: subsystem.map(ustr),
        };
        let files = [
            file("dom/workers/a.cpp", Some("Core/DOM/Workers")),
            file("dom/workers/b.cpp", Some("Core/DOM/Workers")),
            file("dom/events/a.cpp", Some("Core/DOM/Events")),
            file("netwerk/a.cpp", Some("Core/Networking")),
            file("netwerk/http/a.cpp", Some("Core/Networking/HTTP")),
            file("netwerk/dns/a.cpp", Some("Core/Networking/DNS")),
            file("dom/media/gmp/a.cpp", Some("Core/Audio-Video/GMP")),
            file("browser/a.js", Some("Firefox/General")),
            file("x.txt", None),
        ];
        let (facets, data) = file_facets(&files, &PathKinds(vec![]), &[]);
        let subsystem = facets.iter().find(|f| f.key == "subsystem").unwrap();
        fn tree(values: &[FacetValueView]) -> Vec<String> {
            values
                .iter()
                .map(|value| {
                    let nested = tree(&value.values);
                    if nested.is_empty() {
                        format!("{} {}", value.name, value.count)
                    } else {
                        format!("{} {} ({})", value.name, value.count, nested.join(", "))
                    }
                })
                .collect()
        }
        // Components are nested by their "/"s, the biggest first, and a
        // component which others are in is its group's own files.  (A group's
        // only nested value with no more nesting is folded into it, as for
        // directories.)
        assert_eq!(
            tree(&subsystem.values),
            vec![
                "Core 7 (DOM 3 (Workers 2, Events 1), Networking 3 (DNS 1, HTTP 1), Audio-Video/GMP 1)",
                "Firefox :: General 1",
                "Unknown 1",
            ]
        );
        assert!(subsystem.nested);
        assert_eq!(subsystem.values[0].values[1].value, "Core/Networking/");
        let in_facet = |path: &str| -> Vec<String> {
            let facets: serde_json::Value = serde_json::from_str(&data[path].facets).unwrap();
            serde_json::from_value(facets["subsystem"].clone()).unwrap()
        };
        assert_eq!(in_facet("netwerk/a.cpp"), ["Core/", "Core/Networking/"]);
        assert_eq!(
            in_facet("netwerk/http/a.cpp"),
            ["Core/", "Core/Networking/", "Core/Networking/HTTP"]
        );
    }

    #[test]
    fn test_facet_view_shown() {
        let value = |name: &str, count: u32, nested: usize| FacetValueView {
            value: name.to_string(),
            name: name.to_string(),
            count,
            values: (0..nested)
                .map(|i| FacetValueView {
                    value: format!("{}/{}", name, i),
                    name: i.to_string(),
                    count: 1 + i as u32,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let shown = |values: &[FacetValueView]| -> Vec<String> {
            values
                .iter()
                .filter(|value| !value.elided)
                .map(|value| value.name.clone())
                .collect()
        };

        // Of 12 values, the 9 biggest, in their order, and a button for the
        // other 3; and none expanded, with no room.
        let values: Vec<FacetValueView> = (0..12)
            .map(|i| {
                value(
                    &format!("v{}", i),
                    [5, 1, 9, 2, 8, 7, 3, 6, 4, 10, 11, 12][i],
                    2,
                )
            })
            .collect();
        let view = FacetView::new("dir", "Directory".to_string(), values);
        assert_eq!(view.more, 3);
        assert_eq!(
            shown(&view.values),
            ["v0", "v2", "v4", "v5", "v7", "v8", "v9", "v10", "v11"]
        );
        assert!(view.values.iter().all(|v| !v.expanded));

        // The biggest value's nested values show if there's room for at
        // least 3 of them (2 and a button), its biggest if not all of them;
        // and then the next biggest's, if there's still room.
        let view = FacetView::new(
            "subsystem",
            "Subsystem".to_string(),
            vec![value("a", 100, 20), value("b", 10, 2), value("c", 5, 0)],
        );
        let a = &view.values[0];
        assert!(a.expanded);
        assert_eq!(shown(&a.values), ["14", "15", "16", "17", "18", "19"]);
        assert_eq!(a.more, 14);
        // (3 at the top, and 6 and the button in "a", so no room for "b"'s 2.)
        assert!(!view.values[1].expanded);
        assert_eq!(view.more, 0);

        let view = FacetView::new(
            "subsystem",
            "Subsystem".to_string(),
            vec![value("a", 100, 4), value("b", 10, 2), value("c", 5, 0)],
        );
        assert!(view.values[0].expanded && view.values[1].expanded);
        assert_eq!(view.values[0].more, 0);

        // Values named to be expanded are, whatever their number, with their
        // biggest nested values, and their ancestors; the rest of the room
        // goes to the others as usual.
        let mut values = vec![value("a", 100, 20), value("b", 10, 2)];
        values.extend((0..6).map(|i| value(&format!("c{}", i), 5, 0)));
        let view = FacetView::with_expanded(
            "subsystem",
            "Subsystem".to_string(),
            values,
            &["a".to_string()],
        );
        assert!(view.values[0].expanded && !view.values[1].expanded);
        assert_eq!(shown(&view.values[0].values).len(), 9);
        assert_eq!(view.values[0].more, 11);
        let mut nested = value("a", 100, 3);
        nested.values[0] = value("a/0", 1, 2);
        let view = FacetView::with_expanded(
            "subsystem",
            "Subsystem".to_string(),
            vec![nested, value("b", 50, 2)],
            &["a/0/".to_string()],
        );
        assert!(view.values[0].expanded && view.values[0].values[0].expanded);
        assert!(view.values[1].expanded, "with room for b's");

        // Not a smaller value's nested values if the biggest's don't fit.
        let mut values = vec![value("a", 100, 20), value("b", 10, 2)];
        values.extend((0..6).map(|i| value(&format!("c{}", i), 5, 0)));
        let view = FacetView::new("subsystem", "Subsystem".to_string(), values);
        assert!(view.values.iter().all(|v| !v.expanded));
    }
}
