use serde::{Deserialize, Serialize};

#[derive(Eq, PartialEq, Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OntologyPointerKind {
    Strong,
    Unique,
    Weak,
    Raw,
    Ref,
    // Ex: JS::{Handle, Heap, MutableHandle, Rooted}.
    GCRef,
    Contains,
}
