//! Per-tree history configuration which lets us decorate specific source
//! revisions with settings that affect how history is derived, without having
//! to modify the source repository.
//!
//! ## Layout and semantics
//!
//! The configuration is a directory (conventionally `$CONFIG_REPO/$TREE_NAME/history`)
//! containing an optional `config.toml` with settings for the whole history
//! and notes-style files named by the (full) source revision they apply to:
//! `revs/<rev>.toml`.
//!
//! ```toml
//! # config.toml: The first source revision to derive history for.  Its
//! # ancestors are ignored, so it's treated as if it created every file, merge
//! # parents from before it are dropped, and revisions whose parents are all
//! # from before it (ex: the first revision of a branch which forked before
//! # it) are derived from it instead.  This lets us derive history for a window
//! # of a huge repository's history.
//! start = "<full revision>"
//! ```
//!  The attributes a note provides apply to its revision
//! and are inherited by all of its descendants (following first parents) until
//! another note provides attributes.  A note which doesn't provide `attributes`
//! inherits them, which will allow for future notes which only provide one-time
//! hints for their revision (like declaring that a revision converts `.ini`
//! files to `.toml` files).
//!
//! ```toml
//! # A complete snapshot of the history attributes in .gitattributes syntax,
//! # effective from this revision onward.  Omit to inherit.
//! attributes = """
//! tests/tests/mc-analysis/** searchfox-lang=none
//! """
//! ```
//!
//! The effective attributes for a revision are the source repository's own root
//! `.gitattributes` at that revision (considering only `searchfox-*`
//! attributes) followed by the note's attributes, so the note takes precedence.
//! (Nested `.gitattributes` files are not currently supported.)
//!
//! ## Attributes
//!
//! - `searchfox-lang=<lang>`: Tokenize matching files as the given
//!   `LanguageProfile::lang`, ex: `none` for plain text.
//! - `-searchfox-history`: Don't track matching files in history at all.
//!
//! ## Determinism
//!
//! History is a pure function of the source repository, the history
//! configuration, and the tokenizer.  build-syntax-token-tree records the id of
//! each revision's effective note attributes (and the start revision, if any)
//! in its syntax commit and refuses to run if the configuration no longer
//! matches what was recorded for the already processed revisions it builds on
//! (the parents of the revisions it processes) or for already processed
//! revisions which have notes, since that means the history needs to be
//! regenerated from that point.  (Since it doesn't look at all of the processed
//! revisions, removing the note of a processed revision is only detected if
//! the new revisions inherit its attributes.)

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use git2::{ObjectType, Oid};
use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;

use crate::tree_sitter_support::cst_tokenizer::{
    LanguageProfile, default_profile_for_path, profile_for_lang,
};

pub const ATTR_LANG: &str = "searchfox-lang";
pub const ATTR_HISTORY: &str = "searchfox-history";

fn is_searchfox_attr(name: &str) -> bool {
    name.starts_with("searchfox-")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttrValue {
    /// `attr`
    Set,
    /// `-attr`
    Unset,
    /// `attr=value`
    Value(String),
}

struct Rule {
    matcher: GlobMatcher,
    /// None means explicitly unspecified via `!attr`.
    attrs: Vec<(String, Option<AttrValue>)>,
}

/// Parsed `.gitattributes`-format rules.
#[derive(Default)]
pub struct AttributeRules {
    rules: Vec<Rule>,
}

/// Convert a `.gitattributes` pattern to a glob, or None for patterns which
/// can't match files.
fn pattern_to_glob(pattern: &str) -> Option<String> {
    // Patterns matching a directory don't recursively apply to its contents in
    // `.gitattributes`, so they never match files.
    if pattern.ends_with('/') {
        return None;
    }
    match pattern.strip_prefix('/') {
        Some(anchored) => Some(anchored.to_string()),
        // Patterns containing a slash are relative to the `.gitattributes`
        // location; patterns without one match the basename at any depth.
        None if pattern.contains('/') => Some(pattern.to_string()),
        None => Some(format!("**/{}", pattern)),
    }
}

impl AttributeRules {
    /// Parse `.gitattributes`-format text, only retaining attributes for which
    /// `keep` returns true.  Comments, blank lines, and macro definitions are
    /// ignored.  Quoted patterns are not supported.
    pub fn parse(text: &str, keep: impl Fn(&str) -> bool) -> Result<AttributeRules, String> {
        let mut rules = vec![];
        for (line_idx, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("[attr]") {
                continue;
            }
            let mut pieces = line.split_whitespace();
            let pattern = pieces.next().unwrap();
            if pattern.starts_with('"') {
                return Err(format!(
                    "line {}: quoted patterns are not supported",
                    line_idx + 1
                ));
            }
            let mut attrs = vec![];
            for piece in pieces {
                let (name, value) = if let Some(name) = piece.strip_prefix('-') {
                    (name, Some(AttrValue::Unset))
                } else if let Some(name) = piece.strip_prefix('!') {
                    (name, None)
                } else if let Some((name, value)) = piece.split_once('=') {
                    (name, Some(AttrValue::Value(value.to_string())))
                } else {
                    (piece, Some(AttrValue::Set))
                };
                if keep(name) {
                    attrs.push((name.to_string(), value));
                }
            }
            if attrs.is_empty() {
                continue;
            }
            let Some(glob) = pattern_to_glob(pattern) else {
                continue;
            };
            let matcher = GlobBuilder::new(&glob)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|e| format!("line {}: bad pattern {}: {}", line_idx + 1, pattern, e))?
                .compile_matcher();
            rules.push(Rule { matcher, attrs });
        }
        Ok(AttributeRules { rules })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Returns None if no rule matching the path mentions the attribute,
    /// otherwise the value from the last matching rule that mentions it, which
    /// is None if that rule made the attribute explicitly unspecified.
    fn lookup_raw(&self, path: &str, name: &str) -> Option<Option<&AttrValue>> {
        for rule in self.rules.iter().rev() {
            if let Some((_, value)) = rule.attrs.iter().rev().find(|(n, _)| n == name)
                && rule.matcher.is_match(path)
            {
                return Some(value.as_ref());
            }
        }
        None
    }

    pub fn lookup(&self, path: &str, name: &str) -> Option<&AttrValue> {
        self.lookup_raw(path, name).flatten()
    }

    fn for_each_attr(&self, mut f: impl FnMut(&str, Option<&AttrValue>)) {
        for rule in &self.rules {
            for (name, value) in &rule.attrs {
                f(name, value.as_ref());
            }
        }
    }
}

/// The attributes provided by a note, along with their identity.
pub struct AttributeSet {
    /// The git blob id of the attribute text, which is what we record in syntax
    /// commits to detect configuration changes.
    pub id: Oid,
    pub rules: AttributeRules,
}

impl AttributeSet {
    /// Parse and validate note attributes.  Unlike a source repository's
    /// `.gitattributes`, which we don't control, notes must only use known
    /// `searchfox-*` attributes with valid values.
    pub fn parse_note_attributes(text: &str) -> Result<AttributeSet, String> {
        let rules = AttributeRules::parse(text, is_searchfox_attr)?;
        let mut error = None;
        rules.for_each_attr(|name, value| {
            let problem = match (name, value) {
                (ATTR_LANG, Some(AttrValue::Value(lang))) if profile_for_lang(lang).is_none() => {
                    Some(format!("unknown language {:?} for {}", lang, ATTR_LANG))
                }
                (ATTR_LANG, Some(AttrValue::Value(_)) | None) => None,
                (ATTR_LANG, _) => Some(format!("{} requires a value", ATTR_LANG)),
                (ATTR_HISTORY, Some(AttrValue::Value(_))) => {
                    Some(format!("{} does not take a value", ATTR_HISTORY))
                }
                (ATTR_HISTORY, _) => None,
                (other, _) => Some(format!("unknown attribute {}", other)),
            };
            if error.is_none() {
                error = problem;
            }
        });
        if let Some(error) = error {
            return Err(error);
        }
        Ok(AttributeSet {
            id: Oid::hash_object(ObjectType::Blob, text.as_bytes()).unwrap(),
            rules,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RevNoteFile {
    attributes: Option<String>,
}

pub struct RevNote {
    pub rev: Oid,
    /// None if the note inherits its attributes.
    pub attributes: Option<Arc<AttributeSet>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    start: Option<String>,
}

fn parse_full_rev(rev: &str, path: &Path) -> Result<Oid, String> {
    match Oid::from_str(rev) {
        Ok(oid) if rev.len() == 40 || rev.len() == 64 => Ok(oid),
        _ => Err(format!(
            "{}: {:?} isn't a full revision",
            path.display(),
            rev
        )),
    }
}

#[derive(Default)]
pub struct HistoryConfig {
    notes: HashMap<Oid, RevNote>,
    /// The first source revision to derive history for, if not the root(s).
    pub start: Option<Oid>,
}

impl HistoryConfig {
    pub fn empty() -> HistoryConfig {
        HistoryConfig::default()
    }

    /// Load `dir/config.toml` and the notes in `dir/revs/`, both of which are
    /// optional, but `dir` must exist.
    pub fn load(dir: &Path) -> Result<HistoryConfig, String> {
        if !dir.is_dir() {
            return Err(format!("{} isn't a directory", dir.display()));
        }
        let config_path = dir.join("config.toml");
        let start = if config_path.exists() {
            let text = fs::read_to_string(&config_path).map_err(|e| e.to_string())?;
            let file: ConfigFile =
                toml::from_str(&text).map_err(|e| format!("{}: {}", config_path.display(), e))?;
            file.start
                .map(|rev| parse_full_rev(&rev, &config_path))
                .transpose()?
        } else {
            None
        };

        let revs_dir = dir.join("revs");
        let mut notes = HashMap::new();
        let entries = match fs::read_dir(&revs_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(HistoryConfig { notes, start });
            }
            Err(e) => return Err(format!("Unable to read {}: {}", revs_dir.display(), e)),
        };
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let Some(rev_str) = name.strip_suffix(".toml") else {
                continue;
            };
            let rev = parse_full_rev(rev_str, &path).map_err(|_| {
                format!(
                    "{}: history notes must be named by full revision",
                    path.display()
                )
            })?;
            let text = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let file: RevNoteFile =
                toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e))?;
            let attributes = match file.attributes {
                Some(text) => Some(Arc::new(
                    AttributeSet::parse_note_attributes(&text)
                        .map_err(|e| format!("{}: {}", path.display(), e))?,
                )),
                None => None,
            };
            notes.insert(rev, RevNote { rev, attributes });
        }
        Ok(HistoryConfig { notes, start })
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty() && self.start.is_none()
    }

    pub fn note(&self, rev: Oid) -> Option<&RevNote> {
        self.notes.get(&rev)
    }

    pub fn note_revs(&self) -> impl Iterator<Item = Oid> + '_ {
        self.notes.keys().copied()
    }

    /// The note attributes with the given id (as recorded in syntax commits),
    /// if any note still provides them.
    pub fn attributes_by_id(&self, id: Oid) -> Option<Arc<AttributeSet>> {
        self.notes
            .values()
            .filter_map(|note| note.attributes.as_ref())
            .find(|attrs| attrs.id == id)
            .cloned()
    }

    /// Compute the effective note attributes for each revision, where `revs`
    /// must be in topological order (parents before children) and provides
    /// each revision's first parent, which must be in `revs` or have its
    /// effective attributes in `known` (ex: for already processed revisions).
    /// Revisions without a first parent inherit `root_attributes`, which is how
    /// revisions at the start of a history window inherit notes from before
    /// the window.  Returns `known` extended with `revs`.
    pub fn effective_attributes(
        &self,
        revs: &[(Oid, Option<Oid>)],
        known: HashMap<Oid, Option<Arc<AttributeSet>>>,
        root_attributes: Option<Arc<AttributeSet>>,
    ) -> HashMap<Oid, Option<Arc<AttributeSet>>> {
        let mut effective = known;
        effective.reserve(revs.len());
        for (rev, first_parent) in revs {
            let own = self.notes.get(rev).and_then(|n| n.attributes.clone());
            let attrs = match (own, first_parent) {
                (Some(attrs), _) => Some(attrs),
                (None, Some(p)) => effective.get(p).cloned().flatten(),
                (None, None) => root_attributes.clone(),
            };
            effective.insert(*rev, attrs);
        }
        effective
    }
}

/// The attributes in effect for a revision: the source repository's root
/// `.gitattributes` (only `searchfox-*` attributes) followed by the note
/// attributes, which take precedence.
#[derive(Clone, Default)]
pub struct EffectiveAttributes {
    /// Blob id of the repository's `.gitattributes` and its parsed rules.
    pub repo: Option<(Oid, Arc<AttributeRules>)>,
    pub note: Option<Arc<AttributeSet>>,
}

impl EffectiveAttributes {
    /// An identity which changes whenever the effective attributes may change.
    pub fn identity(&self) -> (Option<Oid>, Option<Oid>) {
        (
            self.repo.as_ref().map(|(id, _)| *id),
            self.note.as_ref().map(|n| n.id),
        )
    }

    pub fn lookup(&self, path: &str, name: &str) -> Option<&AttrValue> {
        if let Some(note) = &self.note
            && let Some(value) = note.rules.lookup_raw(path, name)
        {
            return value;
        }
        self.repo
            .as_ref()
            .and_then(|(_, rules)| rules.lookup(path, name))
    }
}

/// Parse a source repository's `.gitattributes` for history purposes.  We don't
/// control these files, so problems are logged and ignored.
pub fn parse_repo_gitattributes(text: &str) -> AttributeRules {
    AttributeRules::parse(text, is_searchfox_attr).unwrap_or_else(|e| {
        log::warn!("Ignoring unparseable .gitattributes: {}", e);
        AttributeRules::default()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LangSource {
    /// The default for the path's extension.
    Extension,
    /// A `searchfox-lang` attribute.
    Attribute,
}

impl LangSource {
    pub fn as_str(self) -> &'static str {
        match self {
            LangSource::Extension => "extension",
            LangSource::Attribute => "attribute",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedLanguage {
    /// Don't track this file in history, either because we can't (binary) or
    /// because of a `-searchfox-history` attribute.
    Skip,
    Tokenize(LanguageProfile, LangSource),
}

/// Determine how to handle a path given the effective attributes.
pub fn resolve_language(path: &str, attrs: &EffectiveAttributes) -> ResolvedLanguage {
    if attrs.lookup(path, ATTR_HISTORY) == Some(&AttrValue::Unset) {
        return ResolvedLanguage::Skip;
    }
    if let Some(AttrValue::Value(lang)) = attrs.lookup(path, ATTR_LANG) {
        match profile_for_lang(lang) {
            Some(profile) => return ResolvedLanguage::Tokenize(profile, LangSource::Attribute),
            // Notes are validated, so this can only come from the repository's
            // .gitattributes, possibly specifying a language we don't know yet.
            None => log::warn!("Ignoring unknown {} {:?} for {}", ATTR_LANG, lang, path),
        }
    }
    match default_profile_for_path(Path::new(path)) {
        Some(profile) => ResolvedLanguage::Tokenize(profile, LangSource::Extension),
        None => ResolvedLanguage::Skip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(text: &str) -> EffectiveAttributes {
        EffectiveAttributes {
            repo: None,
            note: Some(Arc::new(AttributeSet::parse_note_attributes(text).unwrap())),
        }
    }

    fn lang_of(path: &str, attrs: &EffectiveAttributes) -> String {
        match resolve_language(path, attrs) {
            ResolvedLanguage::Skip => "SKIP".to_string(),
            ResolvedLanguage::Tokenize(profile, source) => {
                format!("{}/{}", profile.lang, source.as_str())
            }
        }
    }

    #[test]
    fn test_patterns() {
        let attrs = note(
            "# comment\n\
             *.foo searchfox-lang=none\n\
             /top.cpp searchfox-lang=none\n\
             tests/fixtures/** searchfox-lang=none\n\
             tests/fixtures/keep.cpp !searchfox-lang\n\
             generated/ -searchfox-history\n\
             vendor/** -searchfox-history\n",
        );
        assert_eq!(lang_of("a/b/c.foo", &attrs), "none/attribute");
        assert_eq!(lang_of("c.foo", &attrs), "none/attribute");
        assert_eq!(lang_of("top.cpp", &attrs), "none/attribute");
        assert_eq!(lang_of("sub/top.cpp", &attrs), "cpp/extension");
        assert_eq!(lang_of("tests/fixtures/a/b.cpp", &attrs), "none/attribute");
        // `!attr` returns the attribute to unspecified.
        assert_eq!(lang_of("tests/fixtures/keep.cpp", &attrs), "cpp/extension");
        // Directory patterns don't apply to their contents.
        assert_eq!(lang_of("generated/x.cpp", &attrs), "cpp/extension");
        assert_eq!(lang_of("vendor/x/y.rs", &attrs), "SKIP");
        assert_eq!(lang_of("image.png", &attrs), "SKIP");
        assert_eq!(lang_of("README", &attrs), "none/extension");
    }

    #[test]
    fn test_precedence() {
        let repo = parse_repo_gitattributes(
            "* -text\n*.inc searchfox-lang=cpp\n*.x searchfox-lang=cpp\n*.y searchfox-lang=klingon\n",
        );
        let mut attrs = EffectiveAttributes {
            repo: Some((Oid::ZERO_SHA1, Arc::new(repo))),
            note: None,
        };
        assert_eq!(lang_of("a.inc", &attrs), "cpp/attribute");
        // Unknown languages in the repo's attributes are ignored.
        assert_eq!(lang_of("a.y", &attrs), "none/extension");

        attrs.note = note("*.x !searchfox-lang\n*.inc searchfox-lang=none\n").note;
        // The note takes precedence, including making things unspecified.
        assert_eq!(lang_of("a.inc", &attrs), "none/attribute");
        assert_eq!(lang_of("a.x", &attrs), "none/extension");
    }

    #[test]
    fn test_note_validation() {
        assert!(AttributeSet::parse_note_attributes("*.x searchfox-lang=cpp").is_ok());
        // Non-searchfox attributes are ignored.
        assert!(AttributeSet::parse_note_attributes("*.x -text diff=foo").is_ok());
        for bad in [
            "*.x searchfox-lang=klingon",
            "*.x searchfox-lang",
            "*.x searchfox-languag=cpp",
            "*.x searchfox-history=yes",
            "\"quoted\" searchfox-lang=cpp",
        ] {
            assert!(
                AttributeSet::parse_note_attributes(bad).is_err(),
                "{} should be rejected",
                bad
            );
        }
    }

    #[test]
    fn test_load_and_inheritance() {
        let dir = std::env::temp_dir().join(format!("hb-history-config-{}", std::process::id()));
        let revs = dir.join("revs");
        fs::create_dir_all(&revs).unwrap();
        let (a, b, c, d) = (
            Oid::from_str("1111111111111111111111111111111111111111").unwrap(),
            Oid::from_str("2222222222222222222222222222222222222222").unwrap(),
            Oid::from_str("3333333333333333333333333333333333333333").unwrap(),
            Oid::from_str("4444444444444444444444444444444444444444").unwrap(),
        );
        fs::write(
            revs.join(format!("{}.toml", b)),
            "attributes = \"\"\"\n*.foo searchfox-lang=none\n\"\"\"\n",
        )
        .unwrap();
        // A note without attributes inherits.
        fs::write(revs.join(format!("{}.toml", c)), "").unwrap();
        let config = HistoryConfig::load(&dir).unwrap();
        let effective = config.effective_attributes(
            &[(a, None), (b, Some(a)), (c, Some(b)), (d, Some(c))],
            HashMap::new(),
            None,
        );
        assert!(effective[&a].is_none());
        let b_id = effective[&b].as_ref().unwrap().id;
        assert_eq!(effective[&c].as_ref().unwrap().id, b_id);
        assert_eq!(effective[&d].as_ref().unwrap().id, b_id);
        assert_eq!(config.attributes_by_id(b_id).unwrap().id, b_id);
        assert!(config.attributes_by_id(a).is_none());
        // Revisions without a first parent (ex: the start of a history window)
        // inherit the provided root attributes.
        let effective = config.effective_attributes(
            &[(c, None), (d, Some(c))],
            HashMap::new(),
            effective[&b].clone(),
        );
        assert_eq!(effective[&d].as_ref().unwrap().id, b_id);
        // Revisions can inherit from already processed revisions.
        let effective = config.effective_attributes(
            &[(d, Some(c))],
            HashMap::from([(c, config.attributes_by_id(b_id))]),
            None,
        );
        assert_eq!(effective[&d].as_ref().unwrap().id, b_id);
        assert_eq!(effective.len(), 2);

        // Unknown keys (ex: misspelled or not-yet-supported hints) are errors.
        fs::write(revs.join(format!("{}.toml", d)), "atributes = \"\"\n").unwrap();
        assert!(HistoryConfig::load(&dir).is_err());
        fs::remove_file(revs.join(format!("{}.toml", d))).unwrap();

        // Notes must be named by full revision.
        fs::write(revs.join("abc123.toml"), "").unwrap();
        assert!(HistoryConfig::load(&dir).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_load_start() {
        let dir = std::env::temp_dir().join(format!("hb-history-start-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // A directory without any files is an empty configuration.
        assert!(HistoryConfig::load(&dir).unwrap().is_empty());

        // `revs/` is optional when there's a `config.toml`.
        let start = "1111111111111111111111111111111111111111";
        fs::write(dir.join("config.toml"), format!("start = \"{}\"\n", start)).unwrap();
        let config = HistoryConfig::load(&dir).unwrap();
        assert_eq!(config.start, Some(Oid::from_str(start).unwrap()));
        assert!(!config.is_empty());

        // The start must be a full revision and unknown keys are errors.
        fs::write(dir.join("config.toml"), "start = \"111111\"\n").unwrap();
        assert!(HistoryConfig::load(&dir).is_err());
        fs::write(dir.join("config.toml"), "begin = \"x\"\n").unwrap();
        assert!(HistoryConfig::load(&dir).is_err());

        fs::remove_dir_all(&dir).unwrap();
        assert!(HistoryConfig::load(&dir).is_err());
    }
}
