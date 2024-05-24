use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
#[cfg(not(target_arch = "wasm32"))]
use std::io::Read;

use itertools::Itertools;

#[cfg(not(target_arch = "wasm32"))]
use flate2::read::GzDecoder;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{from_str, from_value, Map, Value};
use serde_repr::*;
use ustr::{ustr, Ustr, UstrMap};

use super::ontology_mapping::OntologyPointerKind;

#[derive(Copy, Clone, Default, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct Location {
    /// 1-base lined-number.
    pub lineno: u32,
    /// 0-based start column, inclusive.
    pub col_start: u32,
    /// 0-based end column, inclusive.
    pub col_end: u32,
}

#[derive(Clone, Default, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct LineRange {
    /// 1-based starting line-number
    pub start_lineno: u32,
    /// 1-based ending line number.
    pub end_lineno: u32,
}

impl LineRange {
    pub fn is_empty(&self) -> bool {
        (self.start_lineno == 0 && self.end_lineno == 0) ||
        (self.start_lineno == u32::MAX && self.end_lineno == u32::MAX)
    }
}

#[derive(Clone, Default, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct SourceRange {
    /// 1-based starting line number, inclusive.
    pub start_lineno: u32,
    /// 0-based starting column number, inclusive.
    pub start_col: u32,
    /// 1-based ending line number, inclusive.
    pub end_lineno: u32,
    /// 0-based ending column number, inclusive.
    pub end_col: u32,
}

impl SourceRange {
    pub fn is_empty(&self) -> bool {
        // we allow both 0 and u32::MAX as sentinel values.
        self.start_lineno == 0 || self.start_lineno == u32::MAX
    }
}

impl SourceRange {
    /// Union the other SourceRange into this SourceRange.
    pub fn union(&mut self, other: SourceRange) {
        // A start_lineno of 0 represents an empty/omitted range.  The range is best effort and
        // so one range might be empty and the other not.
        if other.start_lineno == 0 {
            // Nothing to do if the other range is empty.
            return;
        }
        if self.start_lineno == 0 {
            // Clobber this range with the other range if we were empty.
            self.start_lineno = other.start_lineno;
            self.start_col = other.start_col;
            self.end_lineno = other.end_lineno;
            self.end_col = other.end_col;
            return;
        }

        if other.start_lineno < self.start_lineno {
            self.start_lineno = other.start_lineno;
            self.start_col = other.start_col;
        } else if other.start_lineno == self.start_lineno && other.start_col < self.start_col {
            self.start_col = other.start_col;
        }

        if other.end_lineno > self.end_lineno {
            self.end_lineno = other.end_lineno;
            self.end_col = other.end_col;
        } else if other.end_lineno == self.end_lineno && other.end_col > self.end_col {
            self.end_col = other.end_col;
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WithLocation<T> {
    pub loc: Location,
    #[serde(flatten)]
    pub data: T,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnalysisKind {
    Use,
    Def,
    Assign,
    Decl,
    Forward,
    Idl,
    Alias,
}

impl AnalysisKind {
    pub fn to_ustr(&self) -> Ustr {
        // We could obviously precompute/LAZY_STATIC these
        match self {
            AnalysisKind::Use => ustr("use"),
            AnalysisKind::Def => ustr("def"),
            AnalysisKind::Assign => ustr("assign"),
            AnalysisKind::Decl => ustr("decl"),
            AnalysisKind::Forward => ustr("forward"),
            AnalysisKind::Idl => ustr("idl"),
            AnalysisKind::Alias => ustr("alias"),
        }
    }
}

/// This is intended to help model the self-describing nature of analysis
/// records where we have `"target": 1` at the start of the field.  A normal
/// single-value enum should take up no space... hopefully that's the case for
/// this too despite the involvement of `serde_repr` to encode the value as an
/// int.
#[derive(Debug, PartialEq, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum TargetTag {
    Target = 1,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisTarget {
    pub target: TargetTag,
    pub kind: AnalysisKind,
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    #[serde(default, skip_serializing_if = "Ustr::is_empty")]
    pub context: Ustr,
    #[serde(default, skip_serializing_if = "Ustr::is_empty")]
    pub contextsym: Ustr,
    #[serde(
        rename = "peekRange",
        default,
        skip_serializing_if = "LineRange::is_empty"
    )]
    pub peek_range: LineRange,
    #[serde(rename = "argRanges", default, skip_serializing_if = "Vec::is_empty")]
    pub arg_ranges: Vec<SourceRange>,
}

/// See TargetTag for more info
#[derive(Debug, Eq, PartialEq, Serialize_repr, Deserialize_repr)]
#[repr(u8)]
pub enum StructuredTag {
    Structured = 1,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredSuperInfo {
    #[serde(default)]
    pub sym: Ustr,
    #[serde(default)]
    pub props: Vec<Ustr>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredArgInfo {
    pub name: Ustr,
    #[serde(rename = "type", default)]
    pub type_pretty: Ustr,
    #[serde(rename = "typesym", default)]
    pub type_sym: Ustr,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredMethodInfo {
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    #[serde(default)]
    pub props: Vec<Ustr>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub labels: BTreeSet<Ustr>,
    #[serde(default)]
    pub args: Vec<StructuredArgInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredBitPositionInfo {
    pub begin: u32,
    pub width: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredOverrideInfo {
    #[serde(default)]
    pub sym: Ustr,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredFieldInfo {
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    #[serde(rename = "type", default)]
    pub type_pretty: Ustr,
    #[serde(rename = "typesym", default)]
    pub type_sym: Ustr,
    // XXX this should be made Optional like size_bytes.
    #[serde(rename = "offsetBytes", default)]
    pub offset_bytes: u32,
    #[serde(rename = "bitPositions")]
    pub bit_positions: Option<StructuredBitPositionInfo>,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub labels: BTreeSet<Ustr>,
    #[serde(default, rename = "pointerInfo", skip_serializing_if = "Vec::is_empty")]
    pub pointer_info: Vec<StructuredPointerInfo>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StructuredPointerInfo {
    pub kind: OntologyPointerKind,
    pub sym: Ustr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingSlotKind {
    /// A class that directly implements or will be subclassed.
    Class,
    /// For situations like XPConnect interfaces reflected into JS (and maybe
    /// WebIDL?) where we are describing the symbol that exposes the IDL
    /// interface into the language, but where that symbol is not directly part
    /// of a class hierarchy.  I'm really not sure about the WebIDL case here,
    /// and it probably will want to depend on how we end up implementing the
    /// rest of the UX around here.  For now we will treat this like `Class`
    /// above for most purposes, but this may enable semantic linking to try
    /// and do XPConnect magic.
    InterfaceName,
    /// Callable.
    Method,
    /// A field/attribute/property that has JS XPIDL semantics where we only
    /// have a single symbol name but it could correspond to a property or any
    /// combination of a getter/setter.
    Attribute,
    /// An enum/const which is expected to be a value somehow.
    Const,
    /// An attribute for which we have a distinct symbol for a getter.
    Getter,
    /// An attribute for which we have a distinct symbol for a setter.
    Setter,
    /// An RPC/IPC send method which will have a corresponding Recv counterpart.
    Send,
    /// An RPC/IPC receive method which will have a corresponding Send
    /// counterpart.
    Recv,
    /// Future: Pref symbol specified in a WebIDL `Pref="foo"` annotation.
    ///
    EnablingPref,
    /// Future: Symbol specified in a WebIDL `Func=Class::Method` annotation.
    EnablingFunc,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingSlotLang {
    Cpp,
    JS,
    Rust,
    Jvm,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BindingOwnerLang {
    Idl,
    Cpp,
    JS,
    Rust,
    Jvm,
}

/// The binding slot mechanism is used to describe the exclusive relationship
/// between IDL symbols and their bindings as well as the non-exclusive
/// support relationships like enabling functions.
///
/// This type is used in 2 directions:
/// 1. From the IDL symbols via the "structured" `binding_slots` field.  In this
///    case the origin symbol will have an `impl_kind` of "idl" and the binding
///    slot target symbols will have non-idl values.
/// 2. On a exclusive symbol referenced via `binding_slots`, this type is also
///    used for the optional back-edge to the defining idl symbol.  This will
///    not be used for support slots like enabling functions where the tentative
///    plan is just to let the IDL file emit "uses" of the enabling func for
///    cross-reference purposes.  In this case the structure is indicating the
///    values which describe the relationship from the IDL symbol to the current
///    symbol.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BindingSlotProps {
    #[serde(rename = "slotKind")]
    pub slot_kind: BindingSlotKind,
    #[serde(rename = "slotLang")]
    pub slot_lang: BindingSlotLang,
    #[serde(rename = "ownerLang")]
    pub owner_lang: BindingOwnerLang,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredBindingSlotInfo {
    #[serde(flatten)]
    pub props: BindingSlotProps,
    #[serde(default)]
    pub sym: Ustr,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OntologySlotKind {
    /// For methods like nsIRunnable::Run, any overrides will have this slot
    /// which points at the constructors.  In the future this might be replaced
    /// or accompanied by a `RunnableDispatch` kind.
    ///
    /// Constructors will have the reciprocal `RunnableMethod` slot.
    ///
    /// The `syms` payload will be the list of symbols for the constructors
    /// for the immediate class.  We intentionally do not look up the superclass
    /// chain here, but that would likely be a side effect if the Run method
    /// calls its superclass run method.
    RunnableConstructor,
    /// For constructors of nsIRunnable/similar subclasses, this slot points at
    /// the run methods which will reference this constructor and its siblings
    /// via `RunnableConstructor`.
    RunnableMethod,
}

/// Evolving mechanism that allows trees to define high-level semantics that
/// allow eliding low-level implementation details in favor of expressing the
/// emergent control or data flow as humans understand it.  In particular, we
/// want simple annotations to provide a more useful understanding of the code
/// that raw static analysis would not be able to infer, at least on the level
/// we can currently implement it.
///
/// For example, nsIRunnable is a case where we know that overrides of the Run
/// method result from the construction of a runnable followed by its dispatch.
/// For now, we will just treat the creation of the runnable class as an implied
/// call to its Run method, but in the future with some static analysis combined
/// with limited symbolic execution, we could also track the code that hands
/// the runnable off to a more generic dispatch system.  (In general a core goal
/// is not to get tripped up by infrastructure code that touches everything.)
///
/// ### Ongoing Design Discussion
///
/// #### Locations / Existings Target Records
///
/// An question is how this mechanism should relate to target records which have
/// location information.  Currently these slots don't have any location info,
/// but effectively serve to repurpose existing records' symbols and contextsym.
/// Arguably the edges we are introducing exclusively for graphing purposes
/// should impact hit results in "search" style results.  For our current
/// "runnable" use case, this is something we can reasonably map to how we
/// handle subclasses/superclasses/overrides since we can straightforwardly map
/// to the entire kind slot of the related symbols.
///
/// But for something like handling preferences or observer notifications where
/// we are partitioning uses based on an argument, this would not be sufficient.
/// We would need a way to filter those hits either through labeling we do ahead
/// of time or that we can recompute on the fly from the `OntologySlotInfo` if
/// we use this model.  An alternate approach for those cases would be to
/// introduce synthetic symbols, which had been the hand-waving tentative plan
/// but which did not address the logistical glue layer and the relationship
/// between the low-level symbols versus the high-level symbols and hit results.
///
/// There is a spectrum in this space in terms of what low level symbols can be
/// usefully faceted versus situations where the results would be so voluminous
/// that normal faceting would likely be overwhelmed and there is an argument
/// for a different UI paradigm and pre-computation.  For example, observer
/// notifications have a sufficiently limited domain that faceting is
/// appropriate, but for preferences the domain is so huge and the usage so
/// extensive that a normal faceting UI would be of dubious utility because the
/// user should probably just keep typing if they are interested in a specific
/// preference.
#[derive(Debug, Serialize, Deserialize)]
pub struct OntologySlotInfo {
    #[serde(rename = "slotKind")]
    pub slot_kind: OntologySlotKind,
    /// The symbols
    pub syms: Vec<Ustr>,
}

/// The structured record type extracts out the necessary information to uniquely identify the
/// symbol and what is required for cross-referencing's establishment of hierarchy/links.  The rest
/// of the data in the JSON payload of the record (minus these fields) is re-encoded as a
/// JSON-formatted string.  It's fine to promote things out of the payload into the struct as
/// needed.
///
/// Structured records are merged by choosing one platform rep to be the canonical variant and
/// embedding the other variants observed under a `variants` attribute.  See `analysis.md` and
/// `merge-analyses.rs` for more details.
#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisStructured {
    pub structured: StructuredTag,
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    // XXX Adding this right now for scip-indexer because we're using the analysis
    // rep as the canonical info to provide to the source record, and right now this
    // only exists on source records and fields.
    // TODO: have crossref.rs promote info into this from the source record as
    // appropriate; especially because at least initially in C++ we'll only have
    // this data from the source record.
    // TODO: consider whether we should have type_sym here too.
    pub type_pretty: Option<Ustr>,
    #[serde(default)]
    pub kind: Ustr,
    // Comes from the ConcisePerFileInfo where the structured record was found.
    #[serde(default)]
    pub subsystem: Option<Ustr>,

    #[serde(rename = "parentsym", skip_serializing_if = "Option::is_none")]
    pub parent_sym: Option<Ustr>,
    #[serde(rename = "slotOwner", skip_serializing_if = "Option::is_none")]
    pub slot_owner: Option<StructuredBindingSlotInfo>,

    #[serde(rename = "implKind", default)]
    pub impl_kind: Ustr,

    #[serde(rename = "sizeBytes")]
    pub size_bytes: Option<u32>,

    #[serde(rename = "bindingSlots", default)]
    pub binding_slots: Vec<StructuredBindingSlotInfo>,
    #[serde(rename = "ontologySlots", default)]
    pub ontology_slots: Vec<OntologySlotInfo>,
    #[serde(default)]
    pub supers: Vec<StructuredSuperInfo>,
    #[serde(default)]
    pub methods: Vec<StructuredMethodInfo>,
    // TODO: This really needs to be the union of all fields across all variants
    // to begin with; right now for the layout table we do manual stuff, but
    // this really is not sufficient.
    #[serde(default)]
    pub fields: Vec<StructuredFieldInfo>,
    #[serde(default)]
    pub overrides: Vec<StructuredOverrideInfo>,
    #[serde(default)]
    pub props: Vec<Ustr>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub labels: BTreeSet<Ustr>,

    // ### Derived by cross-referencing
    #[serde(rename = "idlsym", skip_serializing_if = "Option::is_none")]
    pub idl_sym: Option<Ustr>,
    // Note: Originally these (subclasses, overriddenBy) were meant to hold
    // { pretty, sym } for symmetry, but now the code and docs do reflect these
    // as being symbol only.
    #[serde(rename = "subclasses", default, skip_serializing_if = "Vec::is_empty")]
    pub subclass_syms: Vec<Ustr>,
    #[serde(rename = "overriddenBy", default, skip_serializing_if = "Vec::is_empty")]
    pub overridden_by_syms: Vec<Ustr>,

    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl AnalysisStructured {
    // Retrieve the platforms from "extra" if present; this could arguably just
    // be serialized in the first place.
    pub fn platforms(&self) -> Vec<String> {
        match self.extra.get("platforms") {
            Some(val) => from_value(val.clone()).unwrap_or_default(),
            _ => vec![]
        }
    }

    // TODO: As mentioned on `fields`, we need to unify things during crossref
    // otherwise we may be blind to some fields for our fancy magic.
    pub fn fields_across_all_variants(&self) -> Vec<(Vec<String>, Vec<StructuredFieldInfo>)> {
        let variants: Vec<AnalysisStructured> = match self.extra.get("variants") {
            Some(val) => from_value(val.clone()).unwrap_or_default(),
            _ => vec![]
        };
        // XXX at least for things that are subclassed it seems like we can end up with multiple
        // structured representations right now, so we need to keep track of platforms we've seen
        // so we can avoid adding them a subsequent time.
        let mut seen = HashSet::new();
        let main_platforms = self.platforms();
        for p in &main_platforms {
            seen.insert(p.to_owned());
        }
        let mut results = vec![(main_platforms, self.fields.clone())];

        for v in variants {
            let var_platforms = v.platforms();
            // Try and insert the platforms into the seen set; insert returns true
            // if the element is newly inserted.
            if !var_platforms.iter().all(|p| seen.insert(p.to_owned())) {
                continue;
            }
            results.push((var_platforms, v.fields));
        }
        results
    }
}

mod bool_as_int {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(b: &bool, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i8(if *b { 1 } else { 0 })
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<bool, D::Error>
    where
        D: Deserializer<'de>,
    {
        let i = i8::deserialize(deserializer)?;
        Ok(i != 0)
    }
}

/// Workaround for join() not currently working on the Vec<Ustr>
pub fn join_ustr_vec(arr: &Vec<Ustr>, joiner: &str) -> String {
    arr
        .iter()
        .map(|x| x.as_str())
        .collect::<Vec<&str>>()
        .join(joiner)
}

mod comma_delimited_vec {
    use serde::{Deserialize, Deserializer, Serializer};
    use ustr::{ustr, Ustr};

    use super::join_ustr_vec;

    pub fn serialize<S>(arr: &Vec<Ustr>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&join_ustr_vec(arr, ","))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<Ustr>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(s.split(',').map(ustr).collect())
    }
}

/// See TargetTag for more info
#[derive(Serialize_repr, Deserialize_repr, PartialEq, Debug)]
#[repr(u8)]
pub enum SourceTag {
    Source = 1,
}

fn bool_is_false(b: &bool) -> bool {
    !b
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AnalysisSource {
    pub source: SourceTag,
    #[serde(with = "comma_delimited_vec")]
    pub syntax: Vec<Ustr>,
    pub pretty: Ustr,
    #[serde(with = "comma_delimited_vec")]
    pub sym: Vec<Ustr>,
    #[serde(default, with = "bool_as_int", skip_serializing_if = "bool_is_false")]
    pub no_crossref: bool,
    #[serde(
        rename = "nestingRange",
        default,
        skip_serializing_if = "SourceRange::is_empty"
    )]
    pub nesting_range: SourceRange,
    /// For records that have an associated type (and aren't a type), this is the human-readable
    /// representation of the type that may have all kinds of qualifiers that searchfox otherwise
    /// ignores.  Not all records will have this type.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub type_pretty: Option<Ustr>,
    /// For records that have an associated type, we may be able to map the type to a searchfox
    /// symbol, and if so, this is that.  Even if the record has a `type_pretty`, it may not have a
    /// type_sym.
    #[serde(rename = "typesym", skip_serializing_if = "Option::is_none")]
    pub type_sym: Option<Ustr>,
    #[serde(rename = "argRanges", default, skip_serializing_if = "Vec::is_empty")]
    pub arg_ranges: Vec<SourceRange>,
}

impl AnalysisSource {
    /// Merges the `syntax`, `sym`, `no_crossref`, and `nesting_range` fields from `other`
    /// into `self`. The `no_crossref` field can be different sometimes
    /// with different versions of clang being used across different
    /// platforms; in this case we only set `no_crossref` if all the versions
    /// being merged have the `no_crossref` field set.  The `nesting_range` can
    /// vary due to use of the pre-processor, including an extreme case where the
    /// ranges are non-overlapping.  We choose to union these ranges because
    /// `merge-analyses.rs` only merges adjacent source entries so the space
    /// between the ranges should simply be preprocessor directives.
    ///
    /// Also asserts that the `pretty` field is the same because otherwise
    /// the merge doesn't really make sense.
    pub fn merge(&mut self, mut other: AnalysisSource) {
        assert_eq!(self.pretty, other.pretty);
        self.no_crossref &= other.no_crossref;
        self.syntax.append(&mut other.syntax);
        self.syntax.sort();
        self.syntax.dedup();
        // de-duplicate symbols without sorting the symbol list so we can maintain the original
        // ordering which can allow the symbols to go from most-specific to least-specific.  In
        // the face of multiple platforms with completely platform-specific symbols and where each
        // platform has more than one symbol, this doesn't maintain a useful overall order, but the
        // first symbol can still remain useful.  (And given in-order processing of platforms, the
        // choice of first symbol remains stable as long as the indexer's symbol ordering remains
        // stable.)
        //
        // This currently will give precedence to the order in "other" rather than "self", but
        // it's still consistent.
        other.sym.append(&mut self.sym);
        self.sym.extend(other.sym.drain(0..).unique());
        self.nesting_range.union(other.nesting_range);
        // We regrettably have no guarantee that the types are the same, so just pick a type when
        // we have it.
        // I tried to make this idiomatic using "or" to overwrite the type, but it got ugly.
        if let Some(type_pretty) = other.type_pretty {
            self.type_pretty.get_or_insert(type_pretty);
        }
        if let Some(type_sym) = other.type_sym {
            self.type_sym.get_or_insert(type_sym);
        }
    }

    /// Source records' "pretty" field is prefixed with their SyntaxKind.  It's also placed in the
    /// "syntax" sorted array, but that string/array ends up empty when no_crossref is set, so
    /// it's currently easiest to get it from here.
    ///
    /// XXX note that the clang indexer can generate "enum constant" syntax kinds that possess a
    /// space, but that just means we lose the "constant" bit, not that we get confused about the
    /// pretty name.
    pub fn get_syntax_kind(&self) -> Option<&str> {
        // It's a given that we're using a standard ASCII space character.
        return self.pretty.split(' ').next();
    }

    /// Returns the `sym` array joined with ",".  This convenience method exists
    /// because join() doesn't currently work on Ustr.
    pub fn get_joined_syms(&self) -> String {
        join_ustr_vec(&self.sym, ",")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnalysisUnion {
    Target(AnalysisTarget),
    Source(AnalysisSource),
    Structured(AnalysisStructured),
}

pub fn parse_location(loc: &str) -> Location {
    let v: Vec<&str> = loc.split(":").collect();
    let lineno = v[0].parse::<u32>().unwrap();
    let (col_start, col_end) = if v[1].contains("-") {
        let v: Vec<&str> = v[1].split("-").collect();
        (v[0], v[1])
    } else {
        (v[1], v[1])
    };
    let col_start = col_start.parse::<u32>().unwrap();
    let col_end = col_end.parse::<u32>().unwrap();
    Location {
        lineno: lineno,
        col_start: col_start,
        col_end: col_end,
    }
}

impl Serialize for Location {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = if self.col_start == self.col_end {
            format!("{:05}:{}", self.lineno, self.col_start)
        } else {
            format!("{:05}:{}-{}", self.lineno, self.col_start, self.col_end)
        };
        serializer.serialize_str(&s)
    }
}

impl<'de> Deserialize<'de> for Location {
    fn deserialize<D>(deserializer: D) -> Result<Location, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(parse_location(&s))
    }
}

fn parse_line_range(range: &str) -> LineRange {
    let v: Vec<&str> = range.split("-").collect();
    if v.len() != 2 {
        return LineRange::default();
    }
    let start_lineno = v[0].parse::<u32>().unwrap();
    let end_lineno = v[1].parse::<u32>().unwrap();
    LineRange {
        start_lineno,
        end_lineno,
    }
}

impl Serialize for LineRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{}-{}", self.start_lineno, self.end_lineno))
    }
}

impl<'de> Deserialize<'de> for LineRange {
    fn deserialize<D>(deserializer: D) -> Result<LineRange, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(parse_line_range(&s))
    }
}

fn parse_source_range(range: &str) -> SourceRange {
    let v: Vec<&str> = range.split(&['-', ':'][..]).collect();
    if v.len() != 4 {
        return SourceRange::default();
    }
    let start_lineno = v[0].parse::<u32>().unwrap();
    let start_col = v[1].parse::<u32>().unwrap();
    let end_lineno = v[2].parse::<u32>().unwrap();
    let end_col = v[3].parse::<u32>().unwrap();
    SourceRange {
        start_lineno,
        start_col,
        end_lineno,
        end_col,
    }
}

impl Serialize for SourceRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!(
            "{}:{}-{}:{}",
            self.start_lineno, self.start_col, self.end_lineno, self.end_col
        ))
    }
}

impl<'de> Deserialize<'de> for SourceRange {
    fn deserialize<D>(deserializer: D) -> Result<SourceRange, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(parse_source_range(&s))
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn read_analysis<T>(
    filename: &str,
    filter: &mut dyn FnMut(Value, &Location, usize) -> Option<T>,
) -> Vec<WithLocation<Vec<T>>> {
    read_analyses(vec![filename.to_string()].as_slice(), filter)
}

/// Load analysis data for one or more files, sorting and grouping by location, with data payloads
/// transformed via the provided `filter`, resulting in either AnalysisSource records being
/// returned (if `read_source` is provided) or AnalysisTarget (if `read_target`) and other record
/// types being ignored.
///
/// Analysis files ending in .gz will be automatically decompressed as they are
/// read.
///
/// Note that the filter function is invoked as records are read in, which means
/// that the sort order seen by the filter function is the order the file
/// already had.  It's only the return value that's sorted and grouped.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_analyses<T>(
    filenames: &[String],
    filter: &mut dyn FnMut(Value, &Location, usize) -> Option<T>,
) -> Vec<WithLocation<Vec<T>>> {
    let mut result = Vec::new();
    for (i_file, filename) in filenames.into_iter().enumerate() {
        let file = match File::open(filename) {
            Ok(f) => f,
            Err(_) => {
                // TODO: This should be a warning again or have more explicit
                // propagation of this case to callers.  This was reduced from
                // a warning because we have a bunch of cases from
                // mozsearch-mozilla/shared/collapse-generated-files.sh being
                // invoked on "analysis-*/etc" expansions that don't match and
                // so are passed through directly because we're not using
                // "shopt -s nullglob" there.  Also crossref seems to sometimes
                // end up trying to ingest files that aren't there?  Both of
                // these things should be addressed if we want to turn this back
                // into a warning.
                info!("Error trying to open analysis file [{}]", filename);
                continue;
            }
        };
        // An analysis file that ends in .gz is compressed and should be
        // dynamically decompressed.
        let reader: Box<dyn Read> = if filename.ends_with(".gz") {
            Box::new(GzDecoder::new(file))
        } else {
            Box::new(file)
        };
        let reader = BufReader::new(reader);
        let mut lineno = 0;
        for line in reader.lines() {
            let line = line.unwrap();
            lineno += 1;
            let data: serde_json::Result<Value> = from_str(&line);
            let mut data = match data {
                Ok(data) => data,
                Err(e) => {
                    warn!(
                        "Error [{}] trying to read analysis from file [{}] line [{}]: [{}]",
                        e, filename, lineno, &line
                    );
                    continue;
                }
            };
            let obj = data.as_object_mut().unwrap();
            // Destructively pull the "loc" out before passing it into the filter.  This is for
            // read_structured which stores everything it doesn't directly process in `payload`.
            let loc = parse_location(obj.remove("loc").unwrap().as_str().unwrap());
            match filter(data, &loc, i_file) {
                Some(v) => result.push(WithLocation { data: v, loc: loc }),
                None => {}
            }
        }
    }

    result.sort_by(|x1, x2| x1.loc.cmp(&x2.loc));

    let mut result2 = Vec::new();
    let mut last_loc = None;
    let mut last_vec = Vec::new();
    for r in result {
        match last_loc {
            Some(ll) => {
                if ll == r.loc {
                    last_loc = Some(ll);
                } else {
                    result2.push(WithLocation {
                        loc: ll,
                        data: last_vec,
                    });
                    last_vec = Vec::new();
                    last_loc = Some(r.loc);
                }
            }
            None => {
                last_loc = Some(r.loc);
            }
        }
        last_vec.push(r.data);
    }

    match last_loc {
        Some(ll) => result2.push(WithLocation {
            loc: ll,
            data: last_vec,
        }),
        None => {}
    }

    result2
}

pub fn read_target(obj: Value, _loc: &Location, _i_size: usize) -> Option<AnalysisTarget> {
    // XXX this shouldn't be necessary thanks to our tag, so this should be removable
    if obj.get("target").is_none() {
        return None;
    }

    from_value(obj).ok()
}

pub fn read_structured(obj: Value, _loc: &Location, _i_size: usize) -> Option<AnalysisStructured> {
    // XXX this shouldn't be necessary thanks to our tag, so this should be removable
    if obj.get("structured").is_none() {
        return None;
    }

    from_value(obj).ok()
}

pub fn read_source(obj: Value, _loc: &Location, _i_size: usize) -> Option<AnalysisSource> {
    // XXX this shouldn't be necessary thanks to our tag, so this should be removable
    if obj.get("source").is_none() {
        return None;
    }

    from_value(obj).ok()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Jump {
    pub id: Ustr,
    pub path: String,
    pub lineno: u64,
    pub pretty: String,
}

pub fn read_jumps(filename: &str) -> UstrMap<Jump> {
    let file = File::open(filename).unwrap();
    let reader = BufReader::new(&file);
    let mut result = UstrMap::default();
    let mut lineno = 1;
    for line in reader.lines() {
        let line = line.unwrap();
        let data: serde_json::Result<Value> = from_str(&line);
        let data = match data {
            Ok(data) => data,
            Err(_) => panic!("error on line {}: {}", lineno, &line),
        };
        lineno += 1;

        let array = data.as_array().unwrap();
        let id = ustr(array[0].as_str().unwrap());
        let data = Jump {
            id,
            path: array[1].as_str().unwrap().to_string(),
            lineno: array[2].as_u64().unwrap(),
            pretty: array[3].as_str().unwrap().to_string(),
        };

        result.insert(id, data);
    }
    result
}

/// This is the representation format for the path-lines per-kind results we
/// emit into the crossref database.  It is generic over `T` so that we can use
/// T=`Ustr` for easy string-interning in crossref.rs but so that we can also
/// deserialize the results as T=`String` in `cmd_compile_results` where we
/// ingest this format and the manual parsing logic ends up very verbose.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchResult {
    #[serde(rename = "lno")]
    pub lineno: u32,
    pub bounds: (u32, u32),
    pub line: String,
    pub context: Ustr,
    pub contextsym: Ustr,
    // We used to build up "peekLines" which we excerpted from the file here, but
    // this was never surfaced to users.  The plan at the time had been to try
    // and store specific file offsets that could be directly mapped/seeked, but
    // between effective caching of dynamic search results and good experiences
    // with lol_html, it seems like we will soon be able to just excerpt the
    // statically produced HTML efficiently enough through dynamic HTML
    // filtering.
    #[serde(
        rename = "peekRange",
        default,
        skip_serializing_if = "LineRange::is_empty"
    )]
    pub peek_range: LineRange,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PathSearchResult {
    pub path: Ustr,
    pub path_kind: Ustr,
    pub lines: Vec<SearchResult>,
}
