//! Faceting of results, which groups them hierarchically by some aspect of
//! them (ex: the directories of their paths) when there are usefully sized
//! groups; see `ResultFacetRoot`.  Used by the "compile-results" pipeline
//! command for `/query/` results, and by the `/explore/` pages and interdiff
//! summaries (see `format::explore_facets`).

use std::collections::BTreeMap;

use ustr::Ustr;

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
