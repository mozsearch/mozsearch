use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Seek;
use std::io::Write;
use std::path::Path;

extern crate env_logger;
extern crate tools;
use crate::languages::FormatAs;
use tools::config;
use tools::describe;
use tools::file_format::analysis::{read_analysis, read_jumps, read_source};
use tools::find_source_file;
use tools::format::format_file_data;
use tools::git_ops;
use tools::languages;

use tools::output::{PanelItem, PanelSection};

extern crate rustc_serialize;
use rustc_serialize::json;

fn read_json_from_file(path: &str) -> Option<json::Object> {
    let components_file = File::open(path).ok()?;
    let mut reader = BufReader::new(&components_file);
    json::Json::from_reader(&mut reader).ok()?.into_object()
}

/// For a given path, looks up the bugzilla product and component and
/// returns it in a tuple if it could be found. The JSON data format
/// comes from https://searchfox.org/mozilla-central/rev/47edbd91c43db6229cf32d1fc4bae9b325b9e2d0/python/mozbuild/mozbuild/frontend/mach_commands.py#209-223,243
/// and is fairly straightforward.
fn get_bugzilla_component<'a>(
    bugzilla_data: &'a json::Object,
    path: &str,
) -> Option<(&'a str, &'a str)> {
    let mut path_obj = bugzilla_data.get("paths")?;
    for path_component in path.split('/') {
        path_obj = path_obj.as_object()?.get(path_component)?;
    }
    let component_id = path_obj.as_i64()?.to_string();
    let mut result_iter = bugzilla_data
        .get("components")?
        .as_object()?
        .get(&component_id)?
        .as_array()?
        .iter();
    let product = result_iter.next()?.as_string()?;
    let component = result_iter.next()?.as_string()?;
    Some((product, component))
}

fn main() {
    env_logger::init();

    let args: Vec<_> = env::args().collect();
    let (base_args, fname_args) = args.split_at(3);

    let cfg = config::load(&base_args[1], true);
    println!("Config file read");

    let tree_name = &base_args[2];
    let tree_config = cfg.trees.get(tree_name).unwrap();

    let jumps_fname = format!("{}/jumps", tree_config.paths.index_path);
    //let jumps : std::collections::HashMap<String, tools::analysis::Jump> = std::collections::HashMap::new();
    let jumps = read_jumps(&jumps_fname);
    println!("Jumps read");

    let bugzilla_fname = format!("{}/bugzilla-components.json", tree_config.paths.index_path);
    let bugzilla_data = read_json_from_file(&bugzilla_fname);
    if bugzilla_data.is_some() {
        println!("Bugzilla components read");
    } else {
        println!("No bugzilla-components.json file found");
    }

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
        }
        &None => (None, None),
    };

    let head_commit =
        head_oid.and_then(|oid| tree_config.git.as_ref().unwrap().repo.find_commit(oid).ok());

    let mut extension_mapping = HashMap::new();
    extension_mapping.insert("cpp", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("cc", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("cxx", ("header", vec!["h", "hh", "hpp", "hxx"]));
    extension_mapping.insert("h", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hh", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hpp", ("source", vec!["cpp", "cc", "cxx"]));
    extension_mapping.insert("hxx", ("source", vec!["cpp", "cc", "cxx"]));

    let mut diff_cache = git_ops::TreeDiffCache::new();
    for path in fname_args {
        println!("File {}", path);

        let output_fname = format!("{}/file/{}", tree_config.paths.index_path, path);
        let source_fname = find_source_file(
            path,
            &tree_config.paths.files_path,
            &tree_config.paths.objdir_path,
        );

        let format = languages::select_formatting(path);

        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        let source_file = match File::open(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open file");
                continue;
            }
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
            }
            _ => {}
        };

        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &mut read_source);

        let mut input = String::new();
        match reader.read_to_string(&mut input) {
            Ok(_) => {}
            Err(_) => {
                let mut bytes = Vec::new();
                reader.seek(std::io::SeekFrom::Start(0)).unwrap();
                match reader.read_to_end(&mut bytes) {
                    Ok(_) => {
                        input.push_str(&bytes.iter().map(|c| *c as char).collect::<String>());
                    }
                    Err(e) => {
                        println!("Unable to read file: {:?}", e);
                        continue;
                    }
                }
            }
        }

        if let Some(file_description) = describe::describe_file(&input, &path_wrapper, &format) {
            let description_fname =
                format!("{}/description/{}", tree_config.paths.index_path, path);
            let description_file = File::create(description_fname).unwrap();
            let mut desc_writer = BufWriter::new(description_file);
            write!(desc_writer, "{}", file_description).unwrap();
        }

        let extension = path_wrapper
            .extension()
            .unwrap_or(&OsStr::new(""))
            .to_str()
            .unwrap();
        let show_header = match extension_mapping.get(extension) {
            Some(&(ref description, ref try_extensions)) => {
                let mut result = None;
                for try_ext in try_extensions {
                    let try_buf = path_wrapper.with_extension(try_ext);
                    let try_path = try_buf.as_path();
                    if try_path.exists() {
                        let (path_base, _) = path.split_at(path.len() - extension.len() - 1);
                        result = Some((
                            description.to_owned(),
                            format!("/{}/source/{}.{}", tree_name, path_base, try_ext),
                        ));
                        break;
                    }
                }
                result
            }
            None => None,
        };

        let mut panel = vec![];

        let mut source_panel_items = vec![];
        if let Some((other_desc, other_path)) = show_header {
            source_panel_items.push(PanelItem {
                title: format!("Go to {} file", other_desc),
                link: other_path,
                update_link_lineno: false,
                accel_key: None,
            });
        };
        if let Some(ref bugzilla) = bugzilla_data {
            if !path.contains("__GENERATED__") {
                if let Some((product, component)) = get_bugzilla_component(bugzilla, &path) {
                    source_panel_items.push(PanelItem {
                        title: format!("File a bug in {} :: {}", product, component),
                        link: format!(
                            "https://bugzilla.mozilla.org/enter_bug.cgi?product={}&component={}",
                            product.replace("&", "%26"),
                            component.replace("&", "%26")
                        ),
                        update_link_lineno: false,
                        accel_key: None,
                    });
                }
            }
        }

        if !source_panel_items.is_empty() {
            panel.push(PanelSection {
                name: "Source code".to_owned(),
                items: source_panel_items,
            });
        };

        if let Some(oid) = head_oid {
            if !path.contains("__GENERATED__") {
                let mut vcs_panel_items = vec![];
                vcs_panel_items.push(PanelItem {
                    title: "Permalink".to_owned(),
                    link: format!("/{}/rev/{}/{}", tree_name, oid, path),
                    update_link_lineno: true,
                    accel_key: Some('Y'),
                });
                if let Some(ref hg_root) = tree_config.paths.hg_root {
                    vcs_panel_items.push(PanelItem {
                        title: "Log".to_owned(),
                        link: format!("{}/log/tip/{}", hg_root, path),
                        update_link_lineno: false,
                        accel_key: Some('L'),
                    });
                    vcs_panel_items.push(PanelItem {
                        title: "Raw".to_owned(),
                        link: format!("{}/raw-file/tip/{}", hg_root, path),
                        update_link_lineno: false,
                        accel_key: Some('R'),
                    });
                }
                if tree_config.paths.git_blame_path.is_some() {
                    vcs_panel_items.push(PanelItem {
                        title: "Blame".to_owned(),
                        link: "javascript:alert('Hover over the gray bar on the left to see blame information.')".to_owned(),
                        update_link_lineno: false,
                        accel_key: None,
                    });
                }
                panel.push(PanelSection {
                    name: "Revision control".to_owned(),
                    items: vcs_panel_items,
                });
            }
        }

        let mut tools_items = vec![];
        if let Some(ref hg_root) = tree_config.paths.hg_root {
            tools_items.push(PanelItem {
                title: "HG Web".to_owned(),
                link: format!("{}/file/tip/{}", hg_root, path),
                update_link_lineno: true,
                accel_key: None,
            });
        }
        if let Some(ref ccov_root) = tree_config.paths.ccov_root {
            tools_items.push(PanelItem {
                title: "Code Coverage".to_owned(),
                link: format!("{}#revision=latest&path={}", ccov_root, path),
                update_link_lineno: true,
                accel_key: None,
            });
        }
        if let Some(ref dxr_root) = tree_config.paths.dxr_root {
            tools_items.push(PanelItem {
                title: "DXR".to_owned(),
                link: format!("{}/source/{}", dxr_root, path),
                update_link_lineno: true,
                accel_key: None,
            });
        }
        if let Some(ref github) = tree_config.paths.github_repo {
            match Path::new(path).extension().and_then(OsStr::to_str) {
                Some("md") | Some("rst") => {
                    tools_items.push(PanelItem {
                        title: "Rendered view".to_owned(),
                        link: format!(
                            "{}/blob/{}/{}",
                            github,
                            head_oid.map_or("master".to_string(), |x| x.to_string()),
                            path
                        ),
                        update_link_lineno: false,
                        accel_key: None,
                    });
                }
                _ => (),
            };
        }
        if !tools_items.is_empty() {
            panel.push(PanelSection {
                name: "Other Tools".to_owned(),
                items: tools_items,
            });
        }

        format_file_data(
            &cfg,
            tree_name,
            &panel,
            &head_commit,
            &blame_commit,
            path,
            input,
            &jumps,
            &analysis,
            &mut writer,
            Some(&mut diff_cache),
        )
        .unwrap();
    }
}
