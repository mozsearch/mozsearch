use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    iter::FromIterator,
};

use query_parser::{parse, TermValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use toml::value::Table;

#[derive(Deserialize)]
pub struct RepoIngestionConfig {
    #[serde(default)]
    pub file: BTreeMap<String, FileConfig>,
    #[serde(default)]
    pub pathkind: BTreeMap<String, PathKindConfig>,
}

#[derive(Deserialize)]
pub struct PathKindConfig {
    pub name: String,
    #[serde(default)]
    pub default: bool,
    /// The order in which heuristics will be greedily applied.
    pub decision_order: u32,
    /// The order in which results will be displayed.
    pub sort_order: u32,
    pub heuristics: PathKindHeuristics,
}

/// The heuristics mechanism is a way of classifying a file into a pathkind
/// based on its path.  This is secondary to
#[derive(Deserialize)]
pub struct PathKindHeuristics {
    #[serde(default)]
    pub dir_names: Vec<String>,
    #[serde(default)]
    pub dir_suffixes: Vec<String>,
    #[serde(default)]
    pub path_prefixes: Vec<String>,
}

#[derive(Deserialize)]
pub struct FileConfig {
    pub ingestion: FileIngestion,
    #[serde(default)]
    pub concise: BTreeMap<String, String>,
    #[serde(default)]
    pub detailed: BTreeMap<String, String>,
}

#[derive(Deserialize)]
pub struct FileIngestion {
    root: String,
    nesting: String,
    nesting_key: Option<String>,
    value_lookup: Option<String>,
}

pub struct JsonIngestionState {
    pub config: FileConfig,
    pub concise_per_file: BTreeMap<String, BTreeMap<String, Value>>,
    pub detailed_per_file: BTreeMap<String, BTreeMap<String, Value>>,
}

impl JsonIngestionState {
    pub fn new(config_str: &str) -> Result<JsonIngestionState, String> {
        let config: FileConfig = toml::from_str(config_str).map_err(|err| err.to_string())?;

        Ok(JsonIngestionState {
            config,
            concise_per_file: BTreeMap::new(),
            detailed_per_file: BTreeMap::new(),
        })
    }

    /// Destructively ingest the given value for the given filename if we have a configuration
    /// entry for it.  The filename should just be the basename, without any dirname.
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
    pub fn ingest_file_data(
        &mut self,
        filename: &str,
        input_val: &mut Value,
    ) -> Result<(), String> {
        let config = match self.config.file.get(filename) {
            Some(config) => config,
            None => {
                return Err(format!("No config for {}", filename));
            }
        };

        let lookups: BTreeMap<String, Value> = BTreeMap::new();
        if let Some(value_lookup) = config.value_lookup {
            match input_val.pointer_mut(value_lookup) {
                Some(Value::Object(obj)) => {
                    lookups = obj.clone();
                }
                _ => {
                    return Err(format!("Unable to locate value lookup '{}'", value_lookup));
                }
            }
        }

        let root = match input_val.pointer_mut(config.root) {
            Some(v) => v.take(),
            None => {
                return Err(format!("Unable to find root of '{}'", config.root));
            }
        };

        let eval_file_values = |path: &str, file_val: &Value| {
            if !config.concise.is_empty() {
                let concise_storage = self
                    .concise_per_file
                    .entry(path.to_string())
                    .or_insert_with(|| BTreeMap::new());
                for (key, value_path) in config.concise.iter() {
                    if let Some(traversed) = file_val.pointer(value_path) {
                        concise_storage.insert(key.clone(), traversed.clone());
                    }
                }
            }
            if !config.detailed.is_empty() {
                let detailed_storage = self
                    .detailed_per_file
                    .entry(path.to_string())
                    .or_insert_with(|| BTreeMap::new());
                for (key, value_path) in config.detailed.iter() {
                    if let Some(traversed) = file_val.pointer(value_path) {
                        detailed_storage.insert(key.clone(), traversed.clone());
                    }
                }
            }
        };

        let recurse_dir_dict_with_lookup = |path_so_far: &str, cur: Value| -> Result<(), String> {
            if let Some(Value::Object(obj)) = cur {
                for (filename, value) in obj {
                    let path = format!("{}/{}", path_so_far, filename);
                    if value.is_object() {
                        recurse_dir_dict(&path, value)?;
                    } else {
                        let lookup_key = match value {
                            String(s) => s,
                            Number(n) => format!("{}", n),
                            _ => "".to_string(),
                        };
                        if let Some(looked_up_value) = lookups.get(&lookup_key) {
                            eval_file_values(&path, &value);
                        }
                    }
                }
                Ok(())
            } else {
                Err(format!("expected Object at path '{}'", path_so_far))
            }
        };

        match config.nesting.as_str() {
            // bugzilla mapping, uses the lookup
            "hierarchical-dict-dirs-are-dicts-files-are-values" => {
                recurse_dir_dict_with_lookup("", root)
            }
            // code coverage mapping
            "hierarchical-dict-explicit-key" => {
                if let Some(children_key) = &config.ingestion.nesting_key {
                    let recurse_nested_explicit_children =
                        |path_so_far: &str, cur: Value| -> Result<(), String> {
                            eval_file_values(path_so_far, &cur);
                            if let Some(Value::Object(obj)) =
                                cur.get(&children_key).unwrap_or(|| &Value::Null).take()
                            {
                                for (filename, value) in obj {
                                    let path = format!("{}/{}", path_so_far, filename);
                                    recurse_nested_explicit_children(&path, value);
                                }
                                Ok(())
                            } else {
                                Err(format!("expected Object at path '{}'", path_so_far))
                            }
                        };
                    recurse_nested_explicit_children("", root.take())
                } else {
                    Err(format!("nesting_key required for {}", config.nesting))
                }
            }
            "boring-dict-of-arrays" => {
                if let Some(path_key, Some(Value::Object(root_obj))) =
                    (&config.ingestion.nesting_key, root)
                {
                    // The serde Map wrapper lacks `into_values` so we destructure.
                    for (_, result_array_val) in root_obj.into_iter() {
                        if let Some(Value::Array(result_array)) = result_array_val {
                            for val in result_array {
                                if let Some(Value::String(path)) = val.get(path_key).clone() {
                                    eval_file_values(path, &val)?;
                                }
                            }
                        }
                    }
                    Ok(())
                } else {
                    Err(format!("nesting_key required for {}", config.nesting))
                }
            }
            "flat-dir-dict-files-are-keys" => {
                if let Some(children_key, Some(Value::Object(root_obj))) =
                    (&config.ingestion.nesting_key, root.take())
                {
                    for (dir_path, dir_obj) in root_obj.into_iter() {
                        if let Some(Value::Object(file_list_obj)) = dir_obj.get(children_key) {
                            // note: I'm skipping the take() step here because lazy.
                            for (filename, file_contents) in file_list_obj {
                                let path = format!("{}/{}", dir_path, filename);
                                eval_file_values(&path, file_contents)?;
                            }
                        }
                    }
                    Ok(())
                } else {
                    Err(format!("nesting_key required for {}", config.nesting))
                }
            }
        }
    }
}
