use std::fs::File;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;

use itertools::Itertools;

use flate2::read::GzDecoder;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{from_str, from_value, Map, Value};
use serde_repr::*;
use ustr::{ustr, Ustr, UstrMap};

#[derive(Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct Location {
    pub lineno: u32,
    pub col_start: u32,
    pub col_end: u32,
}

#[derive(Clone, Default, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct LineRange {
    pub start_lineno: u32,
    pub end_lineno: u32,
}

impl LineRange {
    pub fn is_empty(&self) -> bool {
        self.start_lineno == 0 && self.end_lineno == 0
    }
}

#[derive(Default, Eq, PartialEq, PartialOrd, Ord, Debug)]
pub struct SourceRange {
    pub start_lineno: u32,
    pub start_col: u32,
    pub end_lineno: u32,
    pub end_col: u32,
}

impl SourceRange {
    pub fn is_empty(&self) -> bool {
        self.start_lineno == 0
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
    IPC,
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
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    #[serde(default)]
    pub props: Vec<Ustr>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredMethodInfo {
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    #[serde(default)]
    pub props: Vec<Ustr>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredBitPositionInfo {
    pub begin: u32,
    pub width: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredOverrideInfo {
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StructuredFieldInfo {
    #[serde(default)]
    pub pretty: Ustr,
    #[serde(default)]
    pub sym: Ustr,
    #[serde(rename = "type", default)]
    pub type_pretty: Ustr,
    #[serde(rename = "typesym", default)]
    pub type_sym: Ustr,
    #[serde(rename = "offsetBytes", default)]
    pub offset_bytes: u32,
    #[serde(rename = "bitPositions")]
    pub bit_positions: Option<StructuredBitPositionInfo>,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: Option<u32>,
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
    #[serde(default)]
    pub kind: Ustr,

    #[serde(rename = "parentsym", skip_serializing_if = "Option::is_none")]
    pub parent_sym: Option<Ustr>,
    #[serde(rename = "srcsym", skip_serializing_if = "Option::is_none")]
    pub src_sym: Option<Ustr>,
    #[serde(rename = "targetsym", skip_serializing_if = "Option::is_none")]
    pub target_sym: Option<Ustr>,

    #[serde(rename = "implKind", default)]
    pub impl_kind: Ustr,

    #[serde(rename = "sizeBytes")]
    pub size_bytes: Option<u32>,

    #[serde(default)]
    pub supers: Vec<StructuredSuperInfo>,
    #[serde(default)]
    pub methods: Vec<StructuredMethodInfo>,
    #[serde(default)]
    pub fields: Vec<StructuredFieldInfo>,
    #[serde(default)]
    pub overrides: Vec<StructuredOverrideInfo>,
    #[serde(default)]
    pub props: Vec<Ustr>,

    // ### Derived by cross-referencing
    #[serde(rename = "idlsym", skip_serializing_if = "Option::is_none")]
    pub idl_sym: Option<Ustr>,
    // Note: Originally these (subclasses, overriddenBy) were meant to hold
    // { pretty, sym } when emitted (and that's how they're documented), but the
    // current router.py assumes this symbol-only approach.
    #[serde(rename = "subclasses", default, skip_serializing_if = "Vec::is_empty")]
    pub subclass_syms: Vec<Ustr>,
    #[serde(rename = "overriddenBy", default, skip_serializing_if = "Vec::is_empty")]
    pub overridden_by_syms: Vec<Ustr>,

    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

mod bool_as_int {
    use serde::{self, Deserialize, Deserializer, Serializer};

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
    use serde::{self, Deserialize, Deserializer, Serializer};
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
pub fn read_analyses<T>(
    filenames: &[String],
    filter: &mut dyn FnMut(Value, &Location, usize) -> Option<T>,
) -> Vec<WithLocation<Vec<T>>> {
    let mut result = Vec::new();
    for (i_file, filename) in filenames.into_iter().enumerate() {
        let file = match File::open(filename) {
            Ok(f) => f,
            Err(_) => {
                warn!("Error trying to open analysis file [{}]", filename);
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
