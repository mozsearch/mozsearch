use std::env;
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::path::Path;

extern crate tools;
use tools::find_source_file;
use tools::analysis::{read_analysis, read_source, read_jumps};
use tools::languages;
use tools::languages::FormatAs;
use tools::format::format_code;

use tools::output::{F, Options, generate_formatted, generate_breadcrumbs, generate_header, generate_footer};

extern crate git2;
use git2::Repository;

fn main() {
    let args: Vec<_> = env::args().collect();
    let (base_args, fname_args) = args.split_at(7);

    let tree_root = &base_args[1];
    //let tree_rev = &base_args[2];
    let index_root = &base_args[3];
    //let mozsearch_root = &base_args[4];
    let objdir = &base_args[5];
    let blame_root = &base_args[6];

    let jumps_fname = index_root.to_string() + "/jumps";
    let jumps : std::collections::HashMap<String, tools::analysis::Jump> = std::collections::HashMap::new();
    //let jumps = read_jumps(&jumps_fname);

    let tree_repo = Repository::open(tree_root).unwrap();
    let blame_repo = Repository::open(blame_root).unwrap();

    let head_oid = blame_repo.refname_to_id("HEAD").unwrap();
    let head_commit = blame_repo.find_commit(head_oid).unwrap();
    let head_tree = head_commit.tree().unwrap();

    for path in fname_args {
        println!("File {}", path);

        let output_fname = format!("{}/file/{}", index_root, path);
        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        let source_fname = find_source_file(path, tree_root, objdir);
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

        let format = languages::select_formatting(path);
        match format {
            FormatAs::Binary => {
                write!(writer, "Binary file").unwrap();
                continue;
            },
            _ => {},
        };

        let blame_data = match head_tree.get_path(Path::new(path)) {
            Ok(tree) => {
                let blame_obj = tree.to_object(&blame_repo).unwrap();
                let blame_blob = blame_obj.as_blob().unwrap();
                let mut content = Vec::new();
                content.extend(blame_blob.content());
                let blame_data = String::from_utf8(content).unwrap();
                Some(blame_data)
            },

            Err(_) => None,
        };
        let blame_lines = if let Some(ref data) = blame_data {
            Some(data.split('\n').collect::<Vec<_>>())
        } else {
            None
        };

        let analysis_fname = format!("{}/analysis/{}", index_root, path);
        let analysis = read_analysis(&analysis_fname, &read_source);

        let mut reader = BufReader::new(&source_file);
        let mut input = String::new();
        match reader.read_to_string(&mut input) {
            Ok(_) => {},
            Err(_) => {
                println!("Unable to read file");
                continue;
            }
        }

        let (output, num_lines, analysis_json) = format_code(&jumps, format, path, &input, &analysis);

        let filename = Path::new(path).file_name().unwrap().to_str().unwrap();
        let title = format!("{} - mozsearch", filename);
        let opt = Options {
            title: &title,
            tree_name: "mozilla-central",
            include_date: true,
        };

        generate_header(&opt, &mut writer).unwrap();

        generate_breadcrumbs(&opt, &mut writer, path).unwrap();

        let f = F::Seq(vec![
            F::S("<table id=\"file\" class=\"file\">"),
            F::Indent(vec![
                F::S("<thead class=\"visually-hidden\">"),
                F::Indent(vec![
                    F::S("<th scope=\"col\">Line</th>"),
                    F::S("<th scope=\"col\">Code</th>"),
                ]),
                F::S("</thead>"),

                F::S("<tbody>"),
                F::Indent(vec![
                    F::S("<tr>"),
                    F::Indent(vec![
                        F::S("<td id=\"line-numbers\">"),
                    ]),
                ]),
            ]),
        ]);

        generate_formatted(&mut writer, &f, 0).unwrap();

        let mut last_rev = None;
        let mut last_color = false;
        let mut strip_id = 0;
        for i in 0 .. num_lines-1 {
            let lineno = i + 1;

            let blame_data = if let Some(ref lines) = blame_lines {
                let blame_line = lines[i as usize];
                let pieces = blame_line.splitn(4, ':').collect::<Vec<_>>();
                let rev = pieces[0];
                let filespec = pieces[1];
                let blame_lineno = pieces[2];
                let filename = if filespec == "%" { &path[..] } else { filespec };

                let color = if last_rev == Some(rev) { last_color } else { !last_color };
                if color != last_color {
                    strip_id += 1;
                }
                last_rev = Some(rev);
                last_color = color;
                let class = if color { 1 } else { 2 };
                let link = format!("/mozilla-central/commit/{}/{}#{}", rev, filename, blame_lineno);
                let data = format!(" class=\"blame-strip c{}\" data-rev=\"{}\" data-link=\"{}\" data-strip=\"{}\"",
                                   class, rev, link, strip_id);

                data
            } else {
                "".to_owned()
            };

            let f = F::Seq(vec![
                F::T(format!("<span id=\"{}\" class=\"line-number\" unselectable=\"on\">{}", lineno, lineno)),
                F::T(format!("<div{}></div>", blame_data)),
                F::S("</span>")
            ]);

            generate_formatted(&mut writer, &f, 0).unwrap();
        }

        let f = F::Seq(vec![
            F::Indent(vec![
                F::Indent(vec![
                    F::Indent(vec![
                        F::S("</td>"),
                        F::S("<td class=\"code\">"),
                    ]),
                ]),
            ]),
        ]);
        generate_formatted(&mut writer, &f, 0).unwrap();
        
        write!(writer, "<pre>").unwrap();
        write!(writer, "{}", output).unwrap();
        write!(writer, "</pre>").unwrap();

        let f = F::Seq(vec![
            F::Indent(vec![
                F::Indent(vec![
                    F::Indent(vec![
                        F::S("</td>"),
                    ]),
                    F::S("</tr>"),
                ]),
                F::S("</tbody>"),
            ]),
            F::S("</table>"),
        ]);
        generate_formatted(&mut writer, &f, 0).unwrap();

        write!(writer, "<script>var ANALYSIS_DATA = {};</script>\n", analysis_json).unwrap();

        generate_footer(&opt, &mut writer).unwrap();
    }
}
