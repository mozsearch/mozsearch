#[macro_use]
extern crate clap;
extern crate rls_analysis as analysis;
extern crate rls_data as data;
extern crate serde;
#[macro_use]
extern crate serde_json;

use analysis::{AnalysisHost, AnalysisLoader};
use std::collections::HashMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

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
    let len = span.column_end.0 - span.column_end.0;
    format!("{}:{}-{}", span.line_start.0, span.column_start.0 - 1, len)
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
    defs: &HashMap<data::Id, data::Def>,
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
            None => continue,
        };

        let def = match defs.get(&id) {
            Some(def) => def,
            None => continue,
        };

        visit(&mut file, "import", &import.span, &def.qualname, None)
    }

    for def in &file_analysis.defs {
        let parent = def.parent
            .and_then(|parent_id| defs.get(&parent_id).map(|d| &*d.qualname));

        visit(&mut file, "def", &def.span, &def.qualname, parent)
    }

    for ref_ in &file_analysis.refs {
        let def = match defs.get(&ref_.ref_id) {
            Some(d) => d,
            None => continue,
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
    defs: &HashMap<data::Id, data::Def>,
    src_dir: &Path,
    output_dir: &Path,
) {
    let mut per_file = HashMap::new();


    macro_rules! flat_map_per_file {
        ($field:ident) => {
            for item in &analysis.$field {
                let mut file_analysis =
                    per_file.entry(item.span.file_name.clone())
                        .or_insert_with(|| {
                            data::Analysis::new(analysis.config.clone())
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


    let crates = analysis::read_analysis_from_files(&loader, Default::default(), &[]);

    let mut defs = HashMap::new();
    for krate in &crates {
        for def in &krate.analysis.defs {
            defs.insert(def.id, def.clone());
        }
    }

    for krate in crates {
        analyze_crate(&krate.analysis, &defs, &src_dir, &output_dir);
    }
}
