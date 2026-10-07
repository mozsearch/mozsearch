use serde::{Deserialize, Serialize};
use ustr::Ustr;

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

pub trait OptionalCrossrefDataHelpers {
    fn structured(&self) -> Option<&AnalysisStructured>;
}

impl OptionalCrossrefDataHelpers for Option<CrossrefData> {
    fn structured(&self) -> Option<&AnalysisStructured> {
        self.as_ref().and_then(|data| data.meta.as_ref())
    }
}
