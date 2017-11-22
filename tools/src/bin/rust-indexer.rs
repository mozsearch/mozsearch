#[macro_use]
extern crate clap;
extern crate rls_analysis;
extern crate rls_data as data;
extern crate serde;
#[macro_use]
extern crate serde_json;

use data::GlobalCrateId;
use rls_analysis::{AnalysisHost, AnalysisLoader};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

/// A global definition id in a crate.
///
/// FIXME(emilio): This key is kind of slow, because GlobalCrateId contains a
/// String. There's a "disambiguator" field which may be more than enough for
/// our purposes.
#[derive(Clone, Hash, Debug, Eq, PartialEq)]
pub struct DefId(GlobalCrateId, u32);

/// A map from global definition ids to the actual definition.
pub struct Defs {
    map: HashMap<DefId, data::Def>,
}

impl Defs {
    fn new() -> Self {
        Self { map: HashMap::new() }
    }

    fn insert(&mut self, analysis: &data::Analysis, def: &data::Def) {
        let crate_id = analysis.prelude.as_ref().unwrap().crate_id.clone();
        let mut definition = def.clone();
        let crate_independent_qualname =
            format!("{}{}", crate_id.name, def.qualname);
        definition.qualname = crate_independent_qualname;

        let index = definition.id.index;
        let previous = self.map.insert(DefId(crate_id, index), definition);
        assert!(
            previous.is_none(),
            "Found a definition with the same ID twice? {:?}",
            def,
        );
    }

    /// Getter for a given local id, which takes care of converting to a global
    /// ID and returning the definition if present.
    fn get(&self, analysis: &data::Analysis, id: data::Id) -> Option<data::Def> {
        let prelude = analysis.prelude.as_ref().unwrap();
        let krate_id = if id.krate == 0 {
            prelude.crate_id.clone()
        } else {
            // TODO(emilio): This escales with the number of crates in this
            // particular crate, but it's probably not too bad, since it should
            // be a pretty fast linear search.
            let krate = prelude.external_crates.iter().find(|krate| {
                krate.num == id.krate
            });

            let krate = match krate {
                Some(k) => k,
                None => {
                    eprintln!("Crate not found: {:?}", id);
                    return None;
                }
            };

            krate.id.clone()
        };

        let id = DefId(krate_id, id.index);
        self.map.get(&id).cloned()
    }
}

#[derive(Clone)]
pub struct Loader {
    deps_dir: PathBuf,
}

impl Loader {
    pub fn new(deps_dir: PathBuf) -> Self {
        Self { deps_dir }
    }
}

impl AnalysisLoader for Loader {
    fn needs_hard_reload(&self, _: &Path) -> bool {
        true
    }

    fn fresh_host(&self) -> AnalysisHost<Self> {
        AnalysisHost::new_with_loader(self.clone())
    }

    fn set_path_prefix(&mut self, _: &Path) {}

    fn abs_path_prefix(&self) -> Option<PathBuf> {
        None
    }
    fn search_directories(&self) -> Vec<PathBuf> {
        vec![self.deps_dir.clone()]
    }
}

// Searchfox uses 1-indexed lines, 0-indexed columns.
fn span_to_string(span: &data::SpanData) -> String {
    // Rust spans are multi-line... So we just set the start column if it spans
    // multiple rows, searchfox has fallback code to handle this.
    if span.line_start != span.line_end {
        return format!("{}:{}", span.line_start.0, span.column_start.0 - 1);
    }
    if span.column_start == span.column_end {
        return format!("{}:{}", span.line_start.0, span.column_start.0 - 1);
    }
    format!("{}:{}-{}", span.line_start.0, span.column_start.0 - 1, span.column_end.0 - 1)
}

fn visit(
    file: &mut File,
    kind: &'static str,
    location: &data::SpanData,
    qualname: &str,
    context: Option<&str>,
) {
    use serde_json::map::Map;
    use serde_json::value::Value;
    use std::io::Write;

    let mut out = Map::new();
    out.insert("loc".into(), Value::String(span_to_string(location)));
    out.insert("target".into(), json!(1));
    out.insert("kind".into(), Value::String(kind.into()));
    out.insert("pretty".into(), Value::String(qualname.into()));
    out.insert("sym".into(), Value::String(qualname.into()));
    if let Some(context) = context {
        out.insert("context".into(), Value::String(context.into()));
        out.insert("contextsym".into(), Value::String(context.into()));
    }

    let object = serde_json::to_string(&Value::Object(out)).unwrap();
    file.write_all(object.as_bytes()).unwrap();
    write!(file, "\n").unwrap();

    let mut out = Map::new();
    out.insert("loc".into(), Value::String(span_to_string(location)));
    out.insert("source".into(), json!(1));
    out.insert("kind".into(), Value::String(kind.into()));
    out.insert("pretty".into(), Value::String(qualname.into()));
    out.insert("sym".into(), Value::String(qualname.into()));

    let object = serde_json::to_string(&Value::Object(out)).unwrap();
    file.write_all(object.as_bytes()).unwrap();
    write!(file, "\n").unwrap();
}

fn analyze_file(
    file_name: &PathBuf,
    defs: &Defs,
    file_analysis: &data::Analysis,
    src_dir: &Path,
    output_dir: &Path,
) {
    let file = match file_name.strip_prefix(src_dir) {
        Ok(f) => f,
        Err(..) => {
            eprintln!("File not in the source directory: {}", file_name.display());
            return;
        }
    };

    let output_file = output_dir.join(file);
    let mut output_dir = output_file.clone();
    output_dir.pop();
    if let Err(err) = fs::create_dir_all(output_dir) {
        eprintln!(
            "Couldn't create dir for: {}, {:?}",
            output_file.display(),
            err
        );
        return;
    }
    let mut file = match File::create(&output_file) {
        Ok(f) => f,
        Err(err) => {
            eprintln!(
                "Couldn't open output file: {}, {:?}",
                output_file.display(),
                err
            );
            return;
        }
    };

    for import in &file_analysis.imports {
        let id = match import.ref_id {
            Some(id) => id,
            None => {
                eprintln!("Dropping import {}: {}, no ref", import.name, import.value);
                continue;
            }
        };

        let def = match defs.get(file_analysis, id) {
            Some(def) => def,
            None => {
                eprintln!("Dropping import {}: {}, no def for ref {:?}", import.name, import.value, id);
                continue;
            }
        };

        visit(&mut file, "use", &import.span, &def.qualname, None)
    }

    for def in &file_analysis.defs {
        let parent = def.parent
            .and_then(|parent_id| defs.get(file_analysis, parent_id).map(|d| d.qualname));

        let crate_name = &file_analysis.prelude.as_ref().unwrap().crate_id.name;
        let qualname = format!("{}{}", crate_name, def.qualname);
        visit(&mut file, "def", &def.span, &qualname, parent.as_ref().map(|p| &**p))
    }

    for ref_ in &file_analysis.refs {
        let def = match defs.get(file_analysis, ref_.ref_id) {
            Some(d) => d,
            None => {
                eprintln!("Dropping ref {:?}, kind {:?}, no def", ref_.ref_id, ref_.kind);
                continue;
            }
        };
        visit(
            &mut file,
            "use",
            &ref_.span,
            &def.qualname,
            /* context = */ None, // TODO
        )
    }
}

fn analyze_crate(
    analysis: &data::Analysis,
    defs: &Defs,
    src_dir: &Path,
    output_dir: &Path,
) {
    let mut per_file = HashMap::new();

    println!("Analyzing crate: {:?}", analysis.prelude);

    macro_rules! flat_map_per_file {
        ($field:ident) => {
            for item in &analysis.$field {
                let mut file_analysis =
                    per_file.entry(item.span.file_name.clone())
                        .or_insert_with(|| {
                            let prelude = analysis.prelude.clone();
                            let mut analysis = data::Analysis::new(analysis.config.clone());
                            analysis.prelude = prelude;
                            analysis
                        });
                file_analysis.$field.push(item.clone());
            }
        }
    }

    flat_map_per_file!(imports);
    flat_map_per_file!(defs);
    flat_map_per_file!(impls);
    flat_map_per_file!(refs);
    flat_map_per_file!(macro_refs);
    flat_map_per_file!(relations);

    for (mut name, analysis) in per_file.drain() {
        if name.is_relative() {
            name = src_dir.join(name);
        }
        analyze_file(&name, defs, &analysis, src_dir, output_dir);
    }
}

fn main() {
    let matches = app_from_crate!()
        .args_from_usage(
            "<src>    'Points to the source root'
             <input>  'Points to the deps/save-analysis directory'
             <output> 'Points to the directory where searchfox metadata should'",
        )
        .get_matches();

    let src_dir = Path::new(matches.value_of("src").unwrap());
    let input_dir = Path::new(matches.value_of("input").unwrap());
    let output_dir = Path::new(matches.value_of("output").unwrap());

    let loader = Loader::new(PathBuf::from(input_dir));


    let crates = rls_analysis::read_analysis_from_files(&loader, Default::default(), &[]);

    let mut defs = Defs::new();
    for krate in &crates {
        for def in &krate.analysis.defs {
            println!("Indexing def: {:?}", def);
            defs.insert(&krate.analysis, def);
        }
    }

    for krate in crates {
        analyze_crate(&krate.analysis, &defs, &src_dir, &output_dir);
    }
}
