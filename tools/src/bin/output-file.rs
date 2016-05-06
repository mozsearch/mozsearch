use std::env;
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
use tools::format::format_text;

use tools::output::*;

fn main() {
    let args: Vec<_> = env::args().collect();
    let (base_args, fname_args) = args.split_at(6);

    let tree_root = &base_args[1];
    //let tree_rev = &base_args[2];
    let index_root = &base_args[3];
    //let mozsearch_root = &base_args[4];
    let objdir = &base_args[5];

    let jumps_fname = index_root.to_string() + "/jumps";
    //let jumps : HashMap<String, tools::analysis::Jump> = HashMap::new();
    let jumps = read_jumps(&jumps_fname);

    for path in fname_args {
        println!("File {}", path);

        let format = languages::select_formatting(path);

        let output_fname = format!("{}/file/{}", index_root, path);
        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        match format {
            FormatAs::Binary => {
                write!(writer, "Binary file").unwrap();
                continue;
            },
            _ => {},
        };

        let analysis_fname = format!("{}/analysis/{}", index_root, path);
        let analysis = read_analysis(&analysis_fname, &read_source);

        let source_fname = find_source_file(path, tree_root, objdir);
        let source_file = match File::open(source_fname) {
            Ok(f) => f,
            Err(_) => {
                println!("Unable to open file");
                continue;
            },
        };
        let mut reader = BufReader::new(&source_file);
        let mut input = String::new();
        match reader.read_to_string(&mut input) {
            Ok(_) => {},
            Err(_) => {
                println!("Unable to read file");
                continue;
            }
        }

        let (output, num_lines, analysis_json) = format_text(&jumps, format, path, &input, &analysis);

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

        for i in 0 .. num_lines {
            write!(writer, "<span id=\"{}\" class=\"line-number\" unselectable=\"on\">{}</span>\n",
                   i + 1, i + 1).unwrap();
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
