use std::env;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::io::Seek;
use std::path::Path;

extern crate tools;
use tools::find_source_file;
use tools::analysis::{read_analysis, read_source, read_jumps};
use tools::format::format_file_data;
use tools::config;

use tools::output::{PanelItem, PanelSection};

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

    let repo = &tree_config.repo;
    let head_oid = repo.refname_to_id("HEAD").unwrap();

    let blame_repo = &tree_config.blame_repo;
    let blame_oid = blame_repo.refname_to_id("HEAD").unwrap();
    let blame_commit = blame_repo.find_commit(blame_oid).unwrap();

    for path in fname_args {
        println!("File {}", path);

        let output_fname = format!("{}/file/{}", tree_config.paths.index_path, path);
        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        let source_fname = find_source_file(path, &tree_config.paths.repo_path, &tree_config.paths.objdir_path);
        let source_file = match File::open(source_fname.clone()) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open file");
                continue;
            },
        };

        let p = Path::new(&source_fname);
        let metadata = fs::symlink_metadata(p).unwrap();
        if metadata.file_type().is_symlink() {
            let dest = fs::read_link(p).unwrap();
            write!(writer, "Symlink to {}", dest.to_str().unwrap()).unwrap();
            continue;
        }

        let analysis_fname = format!("{}/analysis/{}", tree_config.paths.index_path, path);
        let analysis = read_analysis(&analysis_fname, &read_source);

        let mut reader = BufReader::new(&source_file);
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

        let panel = vec![PanelSection {
            name: "Revision control".to_owned(),
            items: vec![PanelItem {
                title: "Permalink".to_owned(),
                link: format!("/{}/rev/{}/{}", tree_name, head_oid, path),
                update_link_lineno: true,
            }, PanelItem {
                title: "Log".to_owned(),
                link: format!("https://hg.mozilla.org/mozilla-central/log/tip/{}", path),
                update_link_lineno: false,
            }, PanelItem {
                title: "Blame".to_owned(),
                link: "javascript:alert('Hover over the gray bar on the left to see blame information.')".to_owned(),
                update_link_lineno: false,
            }],
        }];

        let panel = if path.contains("__GENERATED__") {
            vec![]
        } else {
            panel
        };

        format_file_data(&cfg,
                         tree_name,
                         &panel,
                         None,
                         &blame_commit,
                         path,
                         input,
                         &jumps,
                         &analysis,
                         &mut writer).unwrap();
    }
}
