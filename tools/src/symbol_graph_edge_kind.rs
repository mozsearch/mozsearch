#[derive(Clone)]
pub enum EdgeKind {
    Default, // solid line, closed arrow ("normal")
    // These value are meant to be UML-ish
    Inheritance,    // solid line, open arrow ("onormal")
    Implementation, // dashed line, open arrow ("onormal")
    Composition,    // solid line, closed diamond ("diamond")
    Aggregation,    // solid line, open diamond ("odiamond")
    // These are more specific searchfox concepts
    IPC,           // dotted line, weird vee arrow ("vee")
    CrossLanguage, // JNI-like; solid line, left-half-closed arrow ("lnormal")
}
