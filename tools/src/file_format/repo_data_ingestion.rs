use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fs;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use liquid::Template;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, from_value, json, to_writer, Map, Value};
use ustr::{ustr, Ustr};

use crate::describe::describe_file;
use crate::languages::select_formatting;
use crate::templating::builder::build_and_parse;

use super::config::TreeConfig;
use super::coverage::interpolate_coverage;
use super::globbing_file_list::GlobbingFileList;

#[derive(Deserialize)]
pub struct RepoIngestionConfig {
    #[serde(default)]
    pub textfile: BTreeMap<String, TextFileConfig>,
    #[serde(default)]
    pub jsonfile: BTreeMap<String, JsonFileConfig>,
    #[serde(default)]
    pub pathkind: BTreeMap<Ustr, PathKindConfig>,
}

#[derive(Deserialize)]
pub struct PathKindConfig {
    pub name: Ustr,
    #[serde(default)]
    pub default: bool,
    /// The order in which heuristics will be greedily applied.
    pub decision_order: u32,
    /// The order in which results will be displayed.
    pub sort_order: u32,
    #[serde(default)]
    pub heuristics: PathKindHeuristics,
}

/// The heuristics mechanism is a way of classifying a file into a pathkind
/// based on its path.  This is secondary to any explicit mappings received via
/// explicit lists of files from the textfile/jsonfile mechanisms which will
/// clobber the value computed by these heuristics.
#[derive(Default, Deserialize)]
pub struct PathKindHeuristics {
    #[serde(default)]
    pub dir_names: Vec<String>,
    #[serde(default)]
    pub dir_prefixes: Vec<String>,
    #[serde(default)]
    pub dir_suffixes: Vec<String>,
    #[serde(default)]
    pub path_prefixes: Vec<String>,
}

impl PathKindHeuristics {
    pub fn file_matches<'a, I>(&self, file: &str, dir_segments: I) -> bool
    where
        I: Iterator<Item = &'a str>,
    {
        for prefix in &self.path_prefixes {
            if file.starts_with(prefix) {
                return true;
            }
        }

        for dir in dir_segments {
            if self.dir_names.iter().any(|x| x == dir) {
                return true;
            }

            for dir_prefix in &self.dir_prefixes {
                if dir.starts_with(dir_prefix) {
                    return true;
                }
            }

            for dir_suffix in &self.dir_suffixes {
                if dir.ends_with(dir_suffix) {
                    return true;
                }
            }
        }

        false
    }
}

/// Describes a location a file might be found in a directory tree known to the
/// configuration.
///
/// Supported roots:
/// - `config_repo`: The root of the config repo, this is a checkout of
///   mozsearch-mozilla for searchfox.org.  The tree/repo name does not enter
///   into this
/// - `files`: The tree's source directory with checked out file state, which is
///   usually also its git directory.
/// - `index`: The root of the indexing tree.  This should only be used for
///   files located explicitly in the root.  Use one of the other options
///   instead of specifying a subdirectory.
/// - `mozsearch`: The root of the mozsearch checkout.
/// - `objdir`: The tree's objdir directory.
#[derive(Deserialize)]
pub struct SourceDescriptor {
    pub root: String,
    pub file: String,
}

/// Very limited support for processing text files.  The intent here is to
/// support `.eslintignore` and `.gitignore` style lists as well as very simple
/// "mark all these files as needing data review" in-tree mechanisms that would
/// otherwise potentially require building new `mach` infrastructure (for
/// mozilla-central)
///
#[derive(Deserialize)]
pub struct TextFileConfig {
    pub source: Vec<SourceDescriptor>,
    /// One of the following file formats and associated semantics:
    /// - `file-glob-list`: Passes the list of all files in the repo against the
    ///   `filter_input_ext` extension if present to pre-filter, and then checks
    ///   each remaining file against the contents of the file using
    ///   `.gitignore` semantics where `!` can be used to provide for
    ///   exclusions.
    pub format: String,
    /// Filters the file list that we check against the "file-list" format.
    pub filter_input_ext: Option<Vec<Ustr>>,
    /// A tag to apply to the file if it matched the file list.
    pub apply_tag: Option<Ustr>,
    pub remove_tag: Option<Ustr>,
}

#[derive(Deserialize)]
pub struct JsonFileConfig {
    pub source: Vec<SourceDescriptor>,
    pub ingestion: FileIngestion,
    #[serde(default)]
    pub concise: ConciseIngestion,
    #[serde(default)]
    pub detailed: DetailedIngestion,
}

#[derive(Default, Deserialize)]
pub struct ConciseIngestion {
    pub path_kind: Option<JsonEvalNodeIngestion>,
    pub bugzilla_component: Option<JsonEvalNodeIngestion>,
    pub subsystem: Option<JsonEvalNodeIngestion>,
    #[serde(default)]
    pub info: JsonEvalDictIngestion,
}

#[derive(Default, Deserialize)]
pub struct DetailedIngestion {
    pub coverage_lines: Option<JsonEvalNodeIngestion>,
    #[serde(default)]
    pub info: JsonEvalDictIngestion,
}

pub struct ProbeConfig {
    path: Option<Regex>,
}

impl ProbeConfig {
    pub fn new_from_env() -> Self {
        let path = if let Ok(probe_path) = std::env::var("PROBE_PATH") {
            if let Ok(re_path) = Regex::new(&probe_path) {
                Some(re_path)
            } else {
                None
            }
        } else {
            None
        };

        Self { path }
    }

    pub fn should_probe_path(&self, path: &str) -> bool {
        if let Some(path_regex) = &self.path {
            return path_regex.is_match(path);
        }
        return false;
    }
}

pub struct EvalContext<'a> {
    obj: liquid::Object,
    probe: &'a ProbeConfig,
}

/// Defines an object dictionary's contents.
#[derive(Default, Deserialize)]
pub struct JsonEvalDictIngestion {
    #[serde(flatten)]
    pub extra: BTreeMap<String, JsonEvalNodeIngestion>,
}

impl JsonEvalDictIngestion {
    pub fn eval(
        &mut self,
        ctx: &EvalContext,
        probing: bool,
        input_val: &Value,
        existing_output_value: Value,
    ) -> Value {
        let _obj_entered = if probing {
            let span = Some(trace_span!("dict_eval").entered());
            trace!(existing = ?existing_output_value);
            span
        } else {
            None
        };
        let mut mix_into = match existing_output_value {
            Value::Object(obj) => obj,
            _ => Map::new(),
        };
        for (key, value_ingest) in &mut self.extra {
            if probing {
                trace!(key = %key);
            }
            let existing = mix_into.remove(key).unwrap_or(Value::Null);
            let evaled = value_ingest.eval(&ctx, probing, input_val, existing);

            if !evaled.is_null() {
                if probing {
                    trace!(key, val = ?evaled, "inserting non-null key/value");
                }
                mix_into.insert(key.clone(), evaled);
            } else if probing {
                trace!(key, val = ?evaled, "not inserting null value");
            }
        }
        Value::Object(mix_into)
    }

    pub fn is_empty(&self) -> bool {
        return self.extra.is_empty();
    }
}

/// Defines a mapping transform over arrays only for now.
#[derive(Deserialize)]
pub struct JsonEvalMapIngestion {
    /// Offset to start from.
    pub first_index: usize,
    /// Transform to perform for each value in the array.
    pub each: JsonEvalNodeIngestion,
}

impl JsonEvalMapIngestion {
    pub fn eval(&mut self, ctx: &EvalContext, probing: bool, input_val: Value) -> Value {
        let _obj_entered = if probing {
            Some(trace_span!("map_eval").entered())
        } else {
            None
        };
        match input_val {
            Value::Array(arr) => Value::Array(
                arr.into_iter()
                    .skip(self.first_index)
                    .map(|v| self.each.eval(&ctx, probing, &v, Value::Null))
                    .collect(),
            ),
            _ => Value::Null,
        }
    }
}

/// Defines a mechanism for evaluating a JSON input value and returning a JSON
/// output value, possibly via mutating an existing object dictionary that
/// already exists in the "slot" where this value will be stored.
///
/// Intended to be populated by TOML deserialization.
#[derive(Deserialize)]
pub struct JsonEvalNodeIngestion {
    /// Perform a JSON pointer value of the input value, replacing the input
    /// value for the purposes of processing any subsequent properties.
    /// An empty string returns the value itself.  See
    /// https://docs.rs/serde_json/latest/serde_json/value/enum.Value.html#method.pointer
    /// for more info.
    pub pointer: Option<String>,
    /// Perform a mapping transform over an array.  Evaluated prior to
    /// aggregation.
    pub map: Option<Box<JsonEvalMapIngestion>>,
    /// In the event of a null value, perform this evaluation instead.  This is
    /// being introduced to deal with WPT MANIFEST.json entries which optimize
    /// by omitting test id paths which are the same as the test file path.
    ///
    /// Introducing this does raise the question of whether we should just be
    /// adding support for something like skylark here rather than adding
    /// another specialized case.  I think adding this is appropriate based on
    /// my understanding of the domain, and would probably argue that we should
    /// probably push anything that would resemble embedded scripting upstream
    /// as a step that should be run separate from core searchfox logic.  We're
    /// not doing that in this case because I think an appropriate jq script for
    /// this would still be pretty confusing/complex compared to this
    /// incremental enhancement.
    pub null_fallback: Option<Box<JsonEvalNodeIngestion>>,
    /// Perform some kind of trivial computation on the pointer result from
    /// above.  Current options are:
    /// - "length": Assume we're given an array and get its length.
    pub aggregation: Option<String>,
    /// When present, indicates that the return value should be an object
    /// dictionary, and that the contained key/value definitions should be mixed
    /// in to any already existing object in this slot.
    pub object: Option<JsonEvalDictIngestion>,
    /// When present, indicates that the return value should be a string that
    /// is created by evaluating the payload as a liquid template and evaluating
    /// it with the input value exposed as `value`
    pub liquid: Option<String>,
    #[serde(skip)]
    pub liquid_cache: Option<Template>,
}

impl JsonEvalNodeIngestion {
    pub fn eval(
        &mut self,
        ctx: &EvalContext,
        probing: bool,
        input_val: &Value,
        existing_output_value: Value,
    ) -> Value {
        let _eval_entered = if probing {
            Some(trace_span!("node_eval").entered())
        } else {
            None
        };
        let mut traversed = match &self.pointer {
            Some(traversal) => match input_val.pointer(traversal) {
                Some(val) => {
                    if probing {
                        trace!(traversal, val = ?val, "traversed");
                    }
                    val.clone()
                }
                None => Value::Null,
            },
            None => input_val.clone(),
        };
        if traversed.is_null() {
            if let Some(null_fallback) = &mut self.null_fallback {
                traversed = null_fallback.eval(ctx, probing, &traversed, Value::Null);
                if probing {
                    trace!(val = ?traversed, "null_fallback");
                }
            }
        }
        if let Some(mapper) = &mut self.map {
            traversed = mapper.eval(ctx, probing, traversed);
            if probing {
                trace!(val = ?traversed, "mapped");
            }
        }
        if let Some(aggr) = &self.aggregation {
            traversed = match aggr.as_str() {
                "length" => match traversed {
                    Value::Array(arr) => json!(arr.len()),
                    Value::Object(obj) => json!(obj.len()),
                    _ => Value::Null,
                },
                __ => traversed,
            };
            if probing {
                trace!(val = ?traversed, "length");
            }
        }
        if let Some(object_ingest) = &mut self.object {
            object_ingest.eval(ctx, probing, &traversed, existing_output_value)
        } else if let Some(liquid_str) = &self.liquid {
            let template = self
                .liquid_cache
                .get_or_insert_with(|| build_and_parse(&liquid_str));
            let globals = liquid::object!({
                "value": traversed,
                "context": ctx.obj,
            });
            let rendered = template.render(&globals).unwrap();
            if probing {
                trace!(val = rendered, "rendered");
            }
            Value::String(rendered)
        } else {
            traversed
        }
    }
}

#[derive(Deserialize)]
pub struct FileIngestion {
    root: String,
    nesting: String,
    nesting_key: Option<String>,
    partitioned_by: Option<String>,
    #[serde(default)]
    path_prefix: String,
    filename_key: Option<String>,
    value_lookup: Option<String>,
}

pub struct RepoIngestion {
    pub config: RepoIngestionConfig,
    pub state: IngestionState,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConcisePerFileInfo<T: Ord> {
    pub path_kind: T,
    pub is_dir: bool,
    pub file_size: u64,
    pub bugzilla_component: Option<(T, T)>,
    pub subsystem: Option<T>,
    pub tags: Vec<T>,
    pub description: Option<String>,
    pub info: Value,
}

impl ConcisePerFileInfo<Ustr> {
    fn default_is_dir(is_dir: bool) -> Self {
        ConcisePerFileInfo {
            path_kind: ustr(""),
            is_dir,
            file_size: 0,
            bugzilla_component: None,
            subsystem: None,
            tags: vec![],
            description: None,
            info: json!({}),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub struct DetailedPerFileInfo {
    pub is_dir: bool,
    /// Coverage data; mozilla-central absolutely exceeds i32 regularly.
    pub coverage_lines: Option<Vec<i64>>,
    pub info: Value,
}

impl DetailedPerFileInfo {
    fn default_is_dir(is_dir: bool) -> Self {
        DetailedPerFileInfo {
            is_dir,
            coverage_lines: None,
            info: json!({}),
        }
    }
}

pub struct IngestionState {
    pub concise_per_file: BTreeMap<Ustr, ConcisePerFileInfo<Ustr>>,
    pub detailed_per_file: BTreeMap<Ustr, DetailedPerFileInfo>,
}

fn write_json_to_file<T: Serialize>(val: &T, path: &str) -> Option<()> {
    let file = File::create(path).ok()?;
    let writer = BufWriter::new(file);
    to_writer(writer, val).ok()?;
    Some(())
}

impl IngestionState {
    /// Call the helper function with the concise and detailed storages for the
    /// given path, creating the entries if they do not exist.
    pub fn with_file_info<F>(&mut self, path: &Ustr, is_dir: bool, f: F)
    where
        F: FnOnce(&mut ConcisePerFileInfo<Ustr>, &mut DetailedPerFileInfo),
    {
        let concise_storage = self
            .concise_per_file
            .entry(path.clone())
            .or_insert_with(|| ConcisePerFileInfo::default_is_dir(is_dir));

        let detailed_storage = self
            .detailed_per_file
            .entry(path.clone())
            .or_insert_with(|| DetailedPerFileInfo::default_is_dir(is_dir));

        f(concise_storage, detailed_storage);
    }

    pub fn write_out_concise_file_info(&self, index_path: &str) {
        let output_fname = format!("{}/concise-per-file-info.json", index_path);
        write_json_to_file(&self.concise_per_file, &output_fname);
    }

    pub fn write_out_and_drop_detailed_file_info(&mut self, index_path: &str) {
        for (path, detailed_info) in &self.detailed_per_file {
            let detailed_file_info_fname = if detailed_info.is_dir {
                // We flatten the directories because we already need to do name
                // transforming so we leverage the invariant that there should
                // never be consecutive slashes to normalize them to "_" after
                // doubling existing "_"s.
                format!(
                    "{}/detailed-per-dir-info/{}",
                    index_path,
                    path.replace("_", "__").replace("/", "_")
                )
            } else {
                format!("{}/detailed-per-file-info/{}", index_path, path)
            };

            // We haven't actually bothered to create this directory tree anywhere,
            // and we expect to be sparsely populating it, so just do the mkdir -p
            // ourself here.
            let detailed_path = std::path::Path::new(&detailed_file_info_fname);
            let parent_path = match detailed_path.parent() {
                Some(p) => p,
                None => {
                    warn!("Unable to derive parent of {}", detailed_file_info_fname);
                    continue;
                }
            };
            if let Err(e) = std::fs::create_dir_all(parent_path) {
                warn!(
                    "Problem creating parent of {}: {}",
                    detailed_file_info_fname, e
                );
                continue;
            }

            write_json_to_file(detailed_info, &detailed_file_info_fname);
        }
        self.detailed_per_file.clear();
    }
}

impl RepoIngestion {
    pub fn new(config_str: &str) -> Result<RepoIngestion, String> {
        let config: RepoIngestionConfig =
            toml::from_str(config_str).map_err(|err| err.to_string())?;

        Ok(RepoIngestion {
            config,
            state: IngestionState {
                concise_per_file: BTreeMap::new(),
                detailed_per_file: BTreeMap::new(),
            },
        })
    }

    /// Process the file list of all files (not just files with analysis data)
    /// and apply both path kind heuristics based on the path as well as loading
    /// the file contents to perform trivial processing like having our
    /// `describe_file` mechanism try and derive a snippet.
    ///
    /// The describe mechanism previously happened during the output-file stage.
    pub fn ingest_file_list_and_apply_heuristics(
        &mut self,
        files: &Vec<Ustr>,
        tree_config: &TreeConfig,
    ) {
        let mut ordered_path_kinds: Vec<&PathKindConfig> = self.config.pathkind.values().collect();
        ordered_path_kinds.sort_unstable_by_key(|x| x.decision_order);
        let default_pk = ordered_path_kinds[0].name.clone();

        for file_path in files {
            // split in reverse order so we can skip the filename itself.
            let segments = file_path.rsplit("/").skip(1);
            let mut use_path_kind = default_pk;
            for pk_config in &ordered_path_kinds {
                if pk_config
                    .heuristics
                    .file_matches(file_path, segments.clone())
                {
                    use_path_kind = pk_config.name.clone();
                    break;
                }
            }

            let raw_file_path = tree_config.find_source_file(&file_path);
            let path_wrapper = Path::new(&raw_file_path);
            let metadata = match fs::symlink_metadata(path_wrapper) {
                Ok(m) => m,
                Err(e) => {
                    if tree_config.should_ignore_missing_file(&raw_file_path) {
                        info!("Problem gathering metadata for {}: {}", raw_file_path, e);
                    } else {
                        warn!("Problem gathering metadata for {}: {}", raw_file_path, e);
                    }
                    continue;
                }
            };
            let file_size = metadata.len();

            let description = match fs::read_to_string(&raw_file_path) {
                Ok(contents) => {
                    let format = select_formatting(&raw_file_path);
                    let maybe_description = describe_file(&contents, path_wrapper, &format);
                    if let Some(ref description) = maybe_description {
                        // We currently want to output
                        let description_fname =
                            format!("{}/description/{}", tree_config.paths.index_path, file_path);
                        let description_file = match File::create(&description_fname) {
                            Ok(df) => df,
                            Err(e) => {
                                warn!(
                                    "Problem creating description file {}: {}",
                                    description_fname, e
                                );
                                continue;
                            }
                        };
                        let desc_writer = BufWriter::new(description_file);
                        let file_description = json!({
                            "description": description,
                        });
                        to_writer(desc_writer, &file_description).unwrap();
                    }

                    maybe_description
                }
                Err(_) => None,
            };

            self.state.with_file_info(file_path, false, |pfi, _dfi| {
                pfi.path_kind = use_path_kind;
                pfi.description = description;
                pfi.file_size = file_size;
            });
        }
    }

    pub fn ingest_dir_list(&mut self, dirs: &Vec<Ustr>) {
        for dir_path in dirs {
            self.state.with_file_info(dir_path, true, |_cfi, _dfi| {});
        }
    }

    pub fn ingest_files<F>(&mut self, maybe_read_file: F) -> Result<(), String>
    where
        F: Fn(&str, &str) -> Result<Option<String>, &'static str>,
    {
        let probe_config = ProbeConfig::new_from_env();

        let find_file = |descriptors: &Vec<SourceDescriptor>| -> Result<Option<String>, String> {
            for desc in descriptors {
                match maybe_read_file(&desc.root, &desc.file) {
                    Ok(Some(contents)) => {
                        return Ok(Some(contents));
                    }
                    Ok(None) => {}
                    Err(_) => {
                        return Err(format!(
                            "Problem reading '{}' from root '{}'",
                            desc.file, desc.root
                        ));
                    }
                }
            }

            Ok(None)
        };

        // ### Text Files
        let mut textfile_sources = vec![];
        for (name, config) in &self.config.textfile {
            if let Some(contents) = find_file(&config.source)? {
                textfile_sources.push((name.clone(), contents));
            }
        }
        for (name, contents) in textfile_sources {
            self.ingest_textfile_data(&name, contents, &probe_config)?;
        }

        // ### JSON Files
        let mut jsonfile_sources = vec![];
        for (name, config) in &self.config.jsonfile {
            if let Some(str_contents) = find_file(&config.source)? {
                let val: Value = match from_str(&str_contents) {
                    Err(e) => {
                        return Err(format!("JSON parsing problem for '{}': {:}", name, e));
                    }
                    Ok(v) => v,
                };
                jsonfile_sources.push((name.clone(), val));
            }
        }
        for (name, mut value) in jsonfile_sources {
            self.ingest_jsonfile_data(&name, &mut value, &probe_config)?;
        }

        Ok(())
    }

    pub fn ingest_textfile_data(
        &mut self,
        name: &str,
        file_contents: String,
        probe_config: &ProbeConfig,
    ) -> Result<(), String> {
        let config = match self.config.textfile.get(name) {
            Some(config) => config,
            None => {
                return Err(format!("No config for {}", name));
            }
        };
        info!(
            "Processing text file: {} using format {}",
            name, &config.format
        );

        match config.format.as_str() {
            "file-list" => {
                let globber = GlobbingFileList::new(file_contents);

                for (path, concise) in &mut self.state.concise_per_file {
                    let probing = probe_config.should_probe_path(path);

                    // Apply path extension pre-filter.
                    if let Some(exts) = &config.filter_input_ext {
                        if let Some(idx) = path.rfind('.') {
                            let ext = ustr(&path[idx + 1..]);
                            if exts.iter().any(|x| x == &ext) {
                                // good!
                            } else {
                                continue;
                            }
                        } else {
                            continue;
                        }
                    }

                    // Now apply the glob filter.
                    if !globber.is_match(path) {
                        if probing {
                            trace!("'{}' did not match", path);
                        }
                        continue;
                    } else if probing {
                        trace!("'{}' matched", path);
                    }

                    // It matches if we're here.
                    if let Some(tag) = config.apply_tag {
                        match concise.tags.binary_search(&tag) {
                            Ok(_) => {} // nothing to do, tag already present
                            Err(pos) => {
                                concise.tags.insert(pos, tag.clone());
                            }
                        }
                    }
                    if let Some(tag) = config.remove_tag {
                        match concise.tags.binary_search(&tag) {
                            Ok(pos) => {
                                concise.tags.remove(pos);
                            }
                            Err(_pos) => {} // Nothing to do, tag is not present.
                        }
                    }
                }
                Ok(())
            }
            x => {
                return Err(format!("Unsupported file format '{}'", x));
            }
        }
    }

    /// Destructively ingest the given value for the given filename if we have a configuration
    /// entry for it.  The filename should just be the basename, without any dirname.
    ///
    /// Note: We can probably move this to just take a
    ///
    /// `input_val` is a `&mut Value` which we will use `take()` on, mutating
    /// the JSON in place and then consuming the data-structure to the extent
    /// possible to reduce allocations.  But practically speaking this class is
    /// going to tend to be a bit clone-heavy because of the generic nature of
    /// the concise/detailed data traversals which, although they could `take()`
    /// things, currently clone them because we don't actually care about the
    /// waste enough to impose a "you can only consume each piece of data once"
    /// restriction on the impl.  That seems like the kind of thing that would
    /// be super annoying for people.
    pub fn ingest_jsonfile_data(
        &mut self,
        name: &str,
        input_val: &mut Value,
        probe_config: &ProbeConfig,
    ) -> Result<(), String> {
        info!("Processing JSON file: {}", name);
        let mut config = match self.config.jsonfile.get_mut(name) {
            Some(config) => config,
            None => {
                return Err(format!("No config for {}", name));
            }
        };

        let mut lookups = None;
        if let Some(value_lookup) = &config.ingestion.value_lookup {
            match input_val.pointer_mut(&value_lookup) {
                Some(Value::Object(obj)) => {
                    lookups = Some(obj.clone());
                }
                _ => {
                    return Err(format!("Unable to locate value lookup '{}'", value_lookup));
                }
            }
        }

        let mut root = match input_val.pointer_mut(&config.ingestion.root) {
            Some(v) => v.take(),
            None => {
                return Err(format!(
                    "Unable to find root of '{}'",
                    config.ingestion.root
                ));
            }
        };

        match config.ingestion.nesting.as_str() {
            // Used by:
            // - bugzilla mapping, uses the lookup
            // - wpt MANIFEST.json files, uses the
            "hierarchical-dict-dirs-are-dicts-files-are-values" => {
                let path_prefix = config.ingestion.path_prefix.clone();
                if let Some(_partition_key) = config.ingestion.partitioned_by.clone() {
                    if let Value::Object(obj) = root {
                        for (_, partitioned_root) in obj {
                            self.state.recurse_dir_dict_with_lookup(
                                &mut config,
                                probe_config,
                                &lookups,
                                &path_prefix,
                                partitioned_root,
                            )?;
                        }
                    }
                    Ok(())
                } else {
                    self.state.recurse_dir_dict_with_lookup(
                        &mut config,
                        probe_config,
                        &lookups,
                        &path_prefix,
                        root,
                    )
                }
            }
            // code coverage mapping
            "hierarchical-dict-explicit-key" => {
                if let Some(children_key) = &config.ingestion.nesting_key.clone() {
                    let path_prefix = config.ingestion.path_prefix.clone();
                    self.state.recurse_nested_explicit_children(
                        &mut config,
                        probe_config,
                        children_key,
                        &path_prefix,
                        root.take(),
                    )
                } else {
                    Err(format!(
                        "nesting_key required for {}",
                        config.ingestion.nesting
                    ))
                }
            }
            "boring-dict-of-arrays" => {
                // for the path_prefix, normalize a trailing "/" onto it for
                // consistency with the path_so_far-based mechanisms.
                if let (Some(path_key), path_prefix, Value::Object(root_obj)) = (
                    &config.ingestion.nesting_key.clone(),
                    if config.ingestion.path_prefix.is_empty() {
                        "".to_string()
                    } else {
                        format!("{}/", config.ingestion.path_prefix)
                    },
                    root,
                ) {
                    // The serde Map wrapper lacks `into_values` so we destructure.
                    for (_, result_array_val) in root_obj.into_iter() {
                        if let Value::Array(result_array) = result_array_val {
                            for val in result_array {
                                if let Some(Value::String(path)) = val.get(path_key).clone() {
                                    self.state.eval_file_values(
                                        &mut config,
                                        probe_config,
                                        false,
                                        &ustr(&format!("{}{}", &path_prefix, path)),
                                        false,
                                        false,
                                        &val,
                                    );
                                }
                            }
                        }
                    }
                    Ok(())
                } else {
                    Err(format!(
                        "nesting_key required for {}",
                        config.ingestion.nesting
                    ))
                }
            }
            "flat-dir-dict-files-are-keys" => {
                if let (Some(children_key), filename_key, Value::Object(root_obj)) = (
                    &config.ingestion.nesting_key.clone(),
                    config.ingestion.filename_key.clone(),
                    root.take(),
                ) {
                    // If there's a path_prefix, normalize a trailing "/" onto it
                    // for consistency with our path_so_far-based mechanisms.
                    let use_path_prefix = if config.ingestion.path_prefix.is_empty() {
                        "".to_string()
                    } else {
                        format!("{}/", config.ingestion.path_prefix)
                    };
                    for (dir_path, dir_obj) in root_obj.into_iter() {
                        if let Some(Value::Object(file_list_obj)) = dir_obj.get(children_key) {
                            // note: I'm skipping the take() step here because lazy.
                            for (filename, file_contents) in file_list_obj {
                                let use_filename = match &filename_key {
                                    Some(key) => {
                                        if let Some(Value::String(s)) = file_contents.get(key) {
                                            s
                                        } else {
                                            continue;
                                        }
                                    }
                                    _ => filename,
                                };
                                let path =
                                    format!("{}{}/{}", use_path_prefix, dir_path, use_filename);
                                self.state.eval_file_values(
                                    &mut config,
                                    probe_config,
                                    false,
                                    &ustr(&path),
                                    false,
                                    false,
                                    file_contents,
                                );
                            }
                        }
                    }
                    Ok(())
                } else {
                    Err(format!(
                        "nesting_key required for {}",
                        config.ingestion.nesting
                    ))
                }
            }
            _ => Err(format!(
                "no such nesting strategy: {}",
                config.ingestion.nesting
            )),
        }
    }
}

impl IngestionState {
    pub fn eval_file_values(
        &mut self,
        config: &mut JsonFileConfig,
        probe_config: &ProbeConfig,
        parent_probing: bool,
        path: &Ustr,
        create_if_does_not_exist: bool,
        is_dir: bool,
        file_val: &Value,
    ) {
        let ctx = EvalContext {
            obj: liquid::object!({
                "path": path,
            }),
            probe: probe_config,
        };

        let probing = parent_probing || ctx.probe.should_probe_path(path);

        let concise_entry = self.concise_per_file.entry(path.clone());
        if let Entry::Vacant(_) = &concise_entry {
            if !create_if_does_not_exist {
                return;
            }
        }
        let concise_storage =
            concise_entry.or_insert_with(|| ConcisePerFileInfo::default_is_dir(is_dir));
        if let Some(ingestion) = &mut config.concise.path_kind {
            let evaled = ingestion.eval(&ctx, probing, file_val, Value::Null);
            if !evaled.is_null() {
                concise_storage.path_kind = from_value(evaled).unwrap();
            }
        }
        if let Some(ingestion) = &mut config.concise.bugzilla_component {
            let evaled = ingestion.eval(&ctx, probing, file_val, Value::Null);
            if !evaled.is_null() {
                concise_storage.bugzilla_component = Some(from_value(evaled).unwrap());
            }
        }
        if let Some(ingestion) = &mut config.concise.subsystem {
            let evaled = ingestion.eval(&ctx, probing, file_val, Value::Null);
            if !evaled.is_null() {
                concise_storage.subsystem = Some(from_value(evaled).unwrap());
            }
        }

        concise_storage.info =
            config
                .concise
                .info
                .eval(&ctx, probing, file_val, concise_storage.info.take());

        let detailed_storage = self
            .detailed_per_file
            .entry(path.clone())
            .or_insert_with(|| DetailedPerFileInfo::default_is_dir(is_dir));

        if let Some(ingestion) = &mut config.detailed.coverage_lines {
            let evaled = ingestion.eval(&ctx, probing, file_val, Value::Null);
            if !evaled.is_null() {
                match from_value(evaled) {
                    Ok(converted) => {
                        detailed_storage.coverage_lines = Some(interpolate_coverage(converted));
                    }
                    Err(e) => {
                        warn!(
                            "Weirdness {} on evaluating: {}",
                            e,
                            serde_json::to_string(file_val).unwrap()
                        );
                    }
                }
            }
        }
        detailed_storage.info =
            config
                .detailed
                .info
                .eval(&ctx, probing, file_val, detailed_storage.info.take());
    }

    pub fn recurse_dir_dict_with_lookup(
        &mut self,
        config: &mut JsonFileConfig,
        probe_config: &ProbeConfig,
        lookups: &Option<Map<String, Value>>,
        path_so_far: &str,
        cur: Value,
    ) -> Result<(), String> {
        if let Value::Object(obj) = cur {
            for (filename, value) in obj {
                let path = if path_so_far.is_empty() {
                    filename.clone()
                } else {
                    format!("{}/{}", path_so_far, filename)
                };
                if value.is_object() {
                    self.recurse_dir_dict_with_lookup(config, probe_config, lookups, &path, value)?;
                } else {
                    if let Some(lookup_values) = lookups {
                        let lookup_key = match value {
                            Value::String(s) => s,
                            Value::Number(n) => format!("{}", n),
                            _ => "".to_string(),
                        };
                        if let Some(looked_up_value) = lookup_values.get(&lookup_key) {
                            self.eval_file_values(
                                config,
                                probe_config,
                                false,
                                &ustr(&path),
                                false,
                                false,
                                &looked_up_value,
                            );
                        }
                    } else {
                        self.eval_file_values(
                            config,
                            probe_config,
                            false,
                            &ustr(&path),
                            false,
                            false,
                            &value,
                        );
                    }
                }
            }
            Ok(())
        } else {
            Err(format!("expected Object at path '{}'", path_so_far))
        }
    }

    pub fn recurse_nested_explicit_children(
        &mut self,
        config: &mut JsonFileConfig,
        probe_config: &ProbeConfig,
        children_key: &str,
        path_so_far: &str,
        mut cur: Value,
    ) -> Result<(), String> {
        // We only want to evalute for leaf nodes because we do not currently
        // want to create derived files for directories.  In the future we may
        // want to do so, in which case we will need to explicitly brand the
        // entries as directories and ensure they get mangled to not collide
        // with their own directory.
        //
        // Currently the `children_key` is only for directories and there is no
        // other way to currently distinguish in the coverage file format, so we
        // key off of this.
        if let Some(v) = cur.get_mut(&children_key) {
            if let Value::Object(obj) = v.take() {
                for (filename, value) in obj {
                    let path = if path_so_far.is_empty() {
                        filename.clone()
                    } else {
                        format!("{}/{}", path_so_far, filename)
                    };
                    self.recurse_nested_explicit_children(
                        config,
                        probe_config,
                        children_key,
                        &path,
                        value,
                    )?;
                }
            }
        } else if !path_so_far.is_empty() {
            // (path_so_far would only be empty in this case for a completely
            // empty file, but there's no need to risk doing buggy stuff in that
            // case.)
            self.eval_file_values(
                config,
                probe_config,
                false,
                &ustr(&path_so_far),
                false,
                false,
                &cur,
            );
        }

        Ok(())
    }
}
