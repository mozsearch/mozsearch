//! Faceting of results, which groups them hierarchically by some aspect of
//! them (ex: the directories of their paths) when there are usefully sized
//! groups; see `ResultFacetRoot`.  Used by the "compile-results" pipeline
//! command for `/query/` results, and by the `/explore/` pages and interdiff
//! summaries (see `format::explore_facets`), whose files `file_facets`
//! facets for facet_bar.liquid and facets.js, as for `/query/`'s results.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use serde_json::json;
use ustr::{Ustr, ustr};

use super::interface::{ResultFacetGroup, ResultFacetKind, ResultFacetRoot};

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
            if other.is_none() || clump_miss_count == 0 {
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
            } else {
                let mut other_group = ResultFacetGroup {
                    label: prefix.clone() + other.as_ref().unwrap().as_str(),
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
