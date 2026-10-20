use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

use crate::dataflow::summary::Summary;

use super::{
    analysis::{AnalysisStructured, PathSearchResult},
    ontology_mapping::OntologyPointerKind,
    recency::{Recency, RecencyFrom},
    rollup::{Rollup, RollupDelta},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Callee {
    pub jump: String,
    pub kind: Ustr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pretty: Option<Ustr>,
    pub sym: Ustr,
    /// The tree whose file `jump` is in, if it's a linked tree's (see
    /// `CrossrefData::add_linked`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<Ustr>,
    /// Whether the symbol doesn't call this but implements it: it cites it
    /// (doc-uses it, ex: a Gecko function citing the web spec algorithm it
    /// implements), and it's callable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub implements: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldInfos {
    pub pretty: Ustr,
    pub ptr: OntologyPointerKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FieldMemberUse {
    pub sym: Ustr,
    pub pretty: Ustr,
    pub fields: Vec<FieldInfos>,
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct CrossrefData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uses: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "defs")]
    pub definitions: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "assign")]
    pub assignments: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "decls")]
    pub declarations: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwards: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idl: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "idlp")]
    pub idl_partial: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glean: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "doc-defs")]
    pub doc_defs: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "doc-uses")]
    pub doc_uses: Option<Vec<PathSearchResult>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callees: Option<Vec<Callee>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "field-member-uses"
    )]
    pub field_member_uses: Option<Vec<FieldMemberUse>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<AnalysisStructured>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idl_syms: Option<Vec<Ustr>>,
    /// How recently, and how much, the symbol's code changed, if the tree has
    /// a history (see `hyperblame::recency`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency: Option<Recency>,
    /// Where `recency` is from (see `RecencyFrom`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recency_from: Option<RecencyFrom>,
    /// Candidate scip-typescript symbols if this is a js-analyze symbol
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ts_syms: Vec<Ustr>,
    /// For classes and subsystems: the calls between them and others (see
    /// `rollup`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "callRollup"
    )]
    pub call_rollup: Option<Rollup>,
    /// For classes and subsystems: the data flowing between them and others
    /// (see `rollup`, and `dataflow::summary::FieldEdge`), if the tree has
    /// dataflow summaries.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "dataRollup"
    )]
    pub data_rollup: Option<Rollup>,
    /// For accessors (getters, setters and trivial methods, by their
    /// summaries), if the tree has dataflow summaries: the function's
    /// summary, by which traces step over its calls rather than building its
    /// file's IR (see `dataflow::trace`'s `accessor_summary`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataflow: Option<Summary>,
}

impl CrossrefData {
    /// Whether the symbol is test code: it has definitions, and they're all in
    /// test files (by their path kind; see per-file-info.toml's
    /// `pathkind.test`), as crossref's roll-ups tell (`test_code`).
    pub fn is_test_code(&self) -> bool {
        self.definitions
            .as_ref()
            .is_some_and(|defs| !defs.is_empty() && defs.iter().all(|d| d.path_kind == "test"))
    }

    /// Add a linked tree's data for the symbol (see
    /// `TreeConfigPaths::linked_trees`, ex: a web spec's definitions, for a
    /// symbol cited in Gecko): its path results, in a `tree:<tree>` path kind
    /// with the `tree` (so they're in their own section, from that tree's
    /// files), and its meta and callees, if this has none.
    pub fn add_linked(&mut self, mut linked: CrossrefData, tree: Ustr) {
        linked.tag_tree(tree);
        let add = |own: &mut Option<Vec<PathSearchResult>>,
                   theirs: Option<Vec<PathSearchResult>>| {
            let Some(theirs) = theirs else {
                return;
            };
            own.get_or_insert_with(Vec::new).extend(theirs);
        };
        add(&mut self.uses, linked.uses);
        add(&mut self.definitions, linked.definitions);
        add(&mut self.assignments, linked.assignments);
        add(&mut self.declarations, linked.declarations);
        add(&mut self.forwards, linked.forwards);
        add(&mut self.idl, linked.idl);
        add(&mut self.idl_partial, linked.idl_partial);
        add(&mut self.glean, linked.glean);
        add(&mut self.aliases, linked.aliases);
        add(&mut self.doc_defs, linked.doc_defs);
        add(&mut self.doc_uses, linked.doc_uses);
        if self.meta.is_none() {
            self.meta = linked.meta;
        }
        if self.callees.is_none() {
            self.callees = linked.callees;
        }
    }

    /// Tag a linked tree's data for a symbol with the tree: its path results,
    /// in a `tree:<tree>` path kind with the `tree`, and its callees.
    pub fn tag_tree(&mut self, tree: Ustr) {
        let path_kind = ustr(&format!("tree:{}", tree));
        for results in [
            &mut self.uses,
            &mut self.definitions,
            &mut self.assignments,
            &mut self.declarations,
            &mut self.forwards,
            &mut self.idl,
            &mut self.idl_partial,
            &mut self.glean,
            &mut self.aliases,
            &mut self.doc_defs,
            &mut self.doc_uses,
        ]
        .into_iter()
        .flatten()
        {
            for result in results.iter_mut() {
                result.path_kind = path_kind;
                result.tree = Some(tree);
            }
        }
        for callee in self.callees.iter_mut().flatten() {
            callee.tree = callee.tree.or(Some(tree));
        }
    }
}

/// What a try push changes about a symbol's crossref data (see `try_delta`):
/// its results in the files whose analysis the push changes, which replace the
/// base's results in those files, and, if one of those files has the symbol's
/// structured record (or had), what comes from that.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CrossrefPatch {
    /// The files whose results the push replaces, in every kind of result.
    pub paths: Vec<Ustr>,
    /// The push's results in those files, and with `defined`, the symbol's
    /// structured information, callees, and fields' member uses.
    pub data: CrossrefData,
    /// Whether one of the files has (or had) the symbol's structured record,
    /// so that `data`'s `meta`, `callees`, `field_member_uses`, and `idl_syms`
    /// replace the base's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub defined: bool,
    /// For classes and subsystems: what the push changes about their call
    /// roll-ups (see `RollupDelta`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "callRollupDelta"
    )]
    pub call_rollup_delta: Option<RollupDelta>,
}

impl CrossrefPatch {
    /// The symbol's data with the push's changes (`None` if the push leaves
    /// it none).  The tree's own results are by path, as crossref writes them;
    /// a linked tree's (see `CrossrefData::add_linked`) stay after them.
    pub fn apply(&self, base: Option<CrossrefData>) -> Option<CrossrefData> {
        let mut merged = base.unwrap_or_default();
        let paths: std::collections::HashSet<Ustr> = self.paths.iter().copied().collect();
        let patch = |own: &mut Option<Vec<PathSearchResult>>,
                     theirs: &Option<Vec<PathSearchResult>>| {
            let (mut results, linked): (Vec<_>, Vec<_>) = own
                .take()
                .unwrap_or_default()
                .into_iter()
                .filter(|result| result.tree.is_some() || !paths.contains(&result.path))
                .partition(|result| result.tree.is_none());
            results.extend(theirs.iter().flatten().cloned());
            results.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
            results.extend(linked);
            *own = (!results.is_empty()).then_some(results);
        };
        patch(&mut merged.uses, &self.data.uses);
        patch(&mut merged.definitions, &self.data.definitions);
        patch(&mut merged.assignments, &self.data.assignments);
        patch(&mut merged.declarations, &self.data.declarations);
        patch(&mut merged.forwards, &self.data.forwards);
        patch(&mut merged.idl, &self.data.idl);
        patch(&mut merged.idl_partial, &self.data.idl_partial);
        patch(&mut merged.glean, &self.data.glean);
        patch(&mut merged.aliases, &self.data.aliases);
        patch(&mut merged.doc_defs, &self.data.doc_defs);
        patch(&mut merged.doc_uses, &self.data.doc_uses);
        if self.defined {
            merged.meta = self.data.meta.clone();
            merged.callees = self.data.callees.clone();
            merged.field_member_uses = self.data.field_member_uses.clone();
            merged.idl_syms = self.data.idl_syms.clone();
        }
        if let Some(delta) = &self.call_rollup_delta {
            merged.call_rollup = merged
                .call_rollup
                .take()
                .unwrap_or_default()
                .with_delta(delta);
        }
        let empty = [
            &merged.uses,
            &merged.definitions,
            &merged.assignments,
            &merged.declarations,
            &merged.forwards,
            &merged.idl,
            &merged.idl_partial,
            &merged.glean,
            &merged.aliases,
            &merged.doc_defs,
            &merged.doc_uses,
        ]
        .iter()
        .all(|results| results.is_none())
            && merged.meta.is_none()
            && merged.callees.is_none()
            && merged.call_rollup.is_none();
        (!empty).then_some(merged)
    }
}

pub trait OptionalCrossrefDataHelpers {
    fn structured(&self) -> Option<&AnalysisStructured>;
}

impl OptionalCrossrefDataHelpers for Option<CrossrefData> {
    fn structured(&self) -> Option<&AnalysisStructured> {
        self.as_ref().and_then(|data| data.meta.as_ref())
    }
}
