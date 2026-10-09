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
/// key (as facets.js and URLs name it), label, and values.
#[derive(Serialize)]
pub struct FacetView {
    pub key: &'static str,
    pub label: String,
    pub values: Vec<FacetValueView>,
}

/// A value of a facet (see `ResultFacetGroup`): its identity (its group's
/// label, ex: "dom/base/"), its name relative to its parent's (ex: "base/") and
/// a description, how many files it has, and its nested values.
#[derive(Serialize)]
pub struct FacetValueView {
    pub value: String,
    pub name: String,
    pub title: String,
    pub count: u32,
    pub values: Vec<FacetValueView>,
    /// A sparkline of the value's history digests (see `last_changed_facet`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sparkline: Option<String>,
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
            })
        })
        .collect();
    let file_values = file_values
        .into_iter()
        .map(|(path, values)| (path, values.into_iter().map(String::from).collect()))
        .collect();
    Some((
        FacetView {
            key: "recency",
            label: "Last changed".to_string(),
            values,
        },
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
            view: FacetView {
                key: "pp",
                label: "Preprocessor".to_string(),
                values,
            },
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
/// tree's order), their subsystems (by product and then component), and their
/// directories (the "Path" facet of compile-results), and each file's data
/// for facets.js, by path.
pub fn file_facets(
    files: &[FacetFile],
    kinds: &PathKinds,
) -> (Vec<FacetView>, HashMap<String, FileFacetData>) {
    let kind_order = |kind: &str| kinds.order(kind);
    let kind_name = |kind: &str| kinds.name(kind);

    // Each file's path kind, subsystem, and directory, placed in the facets.
    let mut kind_facet = MaybeFacetRoot::new(ResultFacetKind::PathByKind);
    let mut subsystem_facet = MaybeFacetRoot::new(ResultFacetKind::PathBySubsystem);
    let mut dir_facet = MaybeFacetRoot::new(ResultFacetKind::PathByPath);
    for file in files {
        let path = ustr(&file.path);
        kind_facet.place_item(vec![file.kind], path);
        let subsystem_pieces = match file.subsystem.as_ref().map(|s| s.split_once('/')) {
            Some(Some((product, component))) => {
                vec![ustr(&format!("{}/", product)), ustr(component)]
            }
            Some(None) => vec![file.subsystem.unwrap()],
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
    // Products (ex: "Firefox"), their components (ex: "Sidebar"), and lone
    // components named like Bugzilla does (ex: "Core :: Machine
    // Learning/Frontend").
    let subsystem_name = |value: &str, parent: &str| match value.strip_prefix(parent) {
        _ if value == UNKNOWN_SUBSYSTEM => "Unknown".to_string(),
        Some(component) if !parent.is_empty() => component.to_string(),
        _ => match value.strip_suffix('/') {
            Some(product) => product.to_string(),
            None => value.replacen('/', " :: ", 1),
        },
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
            }
        }
        let mut values: Vec<FacetValueView> = root
            .groups
            .into_iter()
            .map(|group| view(key, group, "", &mut vec![], name, &mut memberships))
            .collect();
        match key {
            "kind" => values.sort_by_key(|value| kind_order(&value.value)),
            "subsystem" => values.sort_by_key(|value| value.value == UNKNOWN_SUBSYSTEM),
            _ => {}
        }
        facets.push(FacetView {
            key,
            label: label.map_or(root.label, str::to_string),
            values,
        });
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
}
