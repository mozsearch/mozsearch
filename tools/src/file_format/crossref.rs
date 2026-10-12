use serde::{Deserialize, Serialize};
use ustr::{Ustr, ustr};

use super::{
    analysis::{AnalysisStructured, PathSearchResult},
    ontology_mapping::OntologyPointerKind,
    recency::{Recency, RecencyFrom},
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
}

impl CrossrefData {
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

pub trait OptionalCrossrefDataHelpers {
    fn structured(&self) -> Option<&AnalysisStructured>;
}

impl OptionalCrossrefDataHelpers for Option<CrossrefData> {
    fn structured(&self) -> Option<&AnalysisStructured> {
        self.as_ref().and_then(|data| data.meta.as_ref())
    }
}
