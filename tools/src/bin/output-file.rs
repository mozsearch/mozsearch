use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::io::Seek;
use std::path::Path;
use std::process::Command;

extern crate tools;
use tools::find_source_file;
use tools::file_format::analysis::{read_analysis, read_source, read_jumps};
use tools::format::format_file_data;
use tools::config;
use tools::languages;
use languages::FormatAs;

use tools::output::{PanelItem, PanelSection};

fn format_documentation(input_fname: &str, output_fname: &str) {
    let _ = Command::new("pandoc")
        .arg("--css")
        .arg("/static/css/pandoc.css")
        .arg("-o")
        .arg(output_fname)
        .arg("-w")
        .arg("html")
        .arg(input_fname)
        .status();
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let (base_args, fname_args) = args.split_at(3);

    let cfg = config::load(&base_args[1], false);
    println!("Config file read");

    let tree_name = &base_args[2];
    let tree_config = cfg.trees.get(tree_name).unwrap();

    let jumps_fname = format!("{}/jumps", tree_config.paths.index_path);
    //let jumps : std::collections::HashMap<String, tools::analysis::Jump> = std::collections::HashMap::new();
    let jumps = read_jumps(&jumps_fname);
    println!("Jumps read");

    let (blame_commit, head_oid) = match &tree_config.git {
        &Some(ref git) => {
            let head_oid = git.repo.refname_to_id("HEAD").unwrap();
            let blame_commit = if let Some(ref blame_repo) = git.blame_repo {
                let blame_oid = blame_repo.refname_to_id("HEAD").unwrap();
                Some(blame_repo.find_commit(blame_oid).unwrap())
            } else {
                None
            };
            (blame_commit, Some(head_oid))
        },
        &None => (None, None),
    };

    for path in fname_args {
        println!("File {}", path);

        let output_fname = format!("{}/file/{}", tree_config.paths.index_path, path);
        let source_fname = find_source_file(path, &tree_config.paths.files_path, &tree_config.paths.objdir_path);

        let format = languages::select_formatting(path);
        match format {
            FormatAs::FormatDoc(_) => {
                let _ = format_documentation(&source_fname, &output_fname);
                continue;
            },
            _ => {},
        };

        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        let source_file = match File::open(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open file");
                continue;
            },
        };

        let path_wrapper = Path::new(&source_fname);
        let metadata = fs::symlink_metadata(path_wrapper).unwrap();
        if metadata.file_type().is_symlink() {
            let dest = fs::read_link(path_wrapper).unwrap();
            write!(writer, "Symlink to {}", dest.to_str().unwrap()).unwrap();
            continue;
        }

        let mut reader = BufReader::new(&source_file);

        match format {
            FormatAs::Binary => {
                let _ = io::copy(&mut reader, &mut writer);
                continue;
            },
            _ => {},
        };

        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &read_source);

        let mut input = String::new();
        match reader.read_to_string(&mut input) {
            Ok(_) => {},
            Err(_) => {
                let mut bytes = Vec::new();
                reader.seek(std::io::SeekFrom::Start(0)).unwrap();
                match reader.read_to_end(&mut bytes) {
                    Ok(_) => {
                        input.push_str(&bytes.iter().map(|c| *c as char).collect::<String>());
                    },
                    Err(e) => {
                        println!("Unable to read file: {:?}", e);
                        continue;
                    }
                }
            }
        }

        let mut extension_mapping = HashMap::new();
        extension_mapping.insert("cpp", ("header", vec!["h", "hh", "hpp", "hxx"]));
        extension_mapping.insert("cc", ("header", vec!["h", "hh", "hpp", "hxx"]));
        extension_mapping.insert("cxx", ("header", vec!["h", "hh", "hpp", "hxx"]));
        extension_mapping.insert("h", ("source", vec!["cpp", "cc", "cxx"]));
        extension_mapping.insert("hh", ("source", vec!["cpp", "cc", "cxx"]));
        extension_mapping.insert("hpp", ("source", vec!["cpp", "cc", "cxx"]));
        extension_mapping.insert("hxx", ("source", vec!["cpp", "cc", "cxx"]));

        let extension = path_wrapper.extension().unwrap_or(&OsStr::new("")).to_str().unwrap();
        let show_header = match extension_mapping.get(extension) {
            Some(&(ref description, ref try_extensions)) => {
                let mut result = None;
                for try_ext in try_extensions {
                    let try_buf = path_wrapper.with_extension(try_ext);
                    let try_path = try_buf.as_path();
                    if try_path.exists() {
                        let (path_base, _) = path.split_at(path.len() - extension.len() - 1);
                        result = Some((description.to_owned(), format!("/{}/source/{}.{}", tree_name, path_base, try_ext)));
                        break;
                    }
                }
                result
            },
            None => {
                None
            }
        };

        let mut panel = if let Some((other_desc, other_path)) = show_header {
            vec![PanelSection {
                name: "Source code".to_owned(),
                items: vec![PanelItem {
                    title: format!("Go to {} file", other_desc),
                    link: other_path,
                    update_link_lineno: false,
                }],
            }]
        } else {
            vec![]
        };

        let mut rev_panel = if path.contains("__GENERATED__") {
            vec![]
        } else if let Some(oid) = head_oid {
            vec![PanelSection {
                name: "Revision control".to_owned(),
                items: vec![PanelItem {
                    title: "Permalink".to_owned(),
                    link: format!("/{}/rev/{}/{}", tree_name, oid, path),
                    update_link_lineno: true,
                }, PanelItem {
                    title: "Log".to_owned(),
                    link: format!("{}/log/tip/{}", config::get_hg_root(tree_config), path),
                    update_link_lineno: false,
                 }, PanelItem {
                    title: "Raw".to_owned(),
                    link: format!("{}/raw-file/tip/{}", config::get_hg_root(tree_config), path),
                    update_link_lineno: false,
                }, PanelItem {
                    title: "Blame".to_owned(),
                    link: "javascript:alert('Hover over the gray bar on the left to see blame information.')".to_owned(),
                    update_link_lineno: false,
                }],
            }]
        } else {
            vec![]
        };

        panel.append(&mut rev_panel);

        let head_commit = head_oid
            .and_then(|oid| tree_config.git.as_ref().unwrap().repo.find_commit(oid).ok());

        format_file_data(&cfg,
                         tree_name,
                         &panel,
                         &head_commit,
                         &blame_commit,
                         path,
                         input,
                         &jumps,
                         &analysis,
                         &mut writer).unwrap();
    }
}
