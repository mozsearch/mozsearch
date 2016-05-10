use std::env;
use std::fs::File;
use std::io::BufReader;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;
use std::collections::HashMap;
use std::path::Path;

extern crate tools;
use tools::find_source_file;
use tools::analysis::{read_analysis, read_source, read_jumps};
use tools::tokenize;

use tools::output::*;

extern crate rustc_serialize;
use self::rustc_serialize::json;
use self::rustc_serialize::json::Json;

enum ShowAs<T> {
    Indexed(T),
    Plain,
    Binary,
}

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

    let reserved_words_js = vec![
        "abstract", "else", "instanceof", "super",
        "boolean", "enum", "int", "switch",
        "break", "export", "interface", "synchronized",
        "byte", "extends", "let", "this",
        "case", "false", "long", "throw",
        "catch", "final", "native", "throws",
        "char", "finally", "new", "transient",
        "class", "float", "null", "true",
        "const", "for", "package", "try",
        "continue", "function", "private", "typeof",
        "debugger", "goto", "protected", "var",
        "default", "if", "public", "void",
        "delete", "implements", "return", "volatile",
        "do", "import", "short", "while",
        "double", "in", "static", "with",
    ];

    let reserved_words_cpp = vec![
        "alignas", "alignof", "and", "and_eq", "asm", "atomic_cancel",
        "atomic_commit", "atomic_noexcept", "auto", "bitand", "bitor", "bool", "break",
        "case", "catch", "char", "char16_t", "char32_t", "class", "compl", "concept",
        "const", "constexpr", "const_cast", "continue", "decltype", "default", "delete",
        "do", "double", "dynamic_cast", "else", "enum", "explicit", "export", "extern", "false",
        "float", "for", "friend", "goto", "if", "inline", "int", "import", "long", "module",
        "mutable", "namespace", "new", "noexcept", "not", "not_eq", "nullptr", "operator",
        "or", "or_eq", "private", "protected", "public", "register", "reinterpret_cast",
        "requires", "return", "short", "signed", "sizeof", "static", "static_assert",
        "static_cast", "struct", "switch", "synchronized", "template", "this", "thread_local",
        "throw", "true", "try", "typedef", "typeid", "typename", "union", "unsigned",
        "using", "virtual", "void", "volatile", "wchar_t", "while", "xor", "xor_eq",
        "#if", "#ifdef", "#ifndef", "#else", "#elif", "#endif", "#define", "#undef",
        "#include", "#error", "defined",
    ];

    let reserved_words_ipdl = vec![
        "answer", "as", "async", "both", "bridges", "call", "child", "class",
        "compress", "compressall", "__delete__", "delete", "from", "goto", "high",
        "include", "intr", "manager", "manages", "namespace", "normal", "nullable",
        "opens", "or", "parent", "prio", "protocol", "recv", "returns", "send",
        "spawns", "start", "state", "struct", "sync", "union", "upto", "urgent",
        "using",
    ];

    let reserved_words_idl = vec![
        "const", "interface", "in", "inout", "out", "attribute", "raises",
        "readonly", "native", "typedef",
        "array", "shared", "iid_is", "size_is", "retval",
        "boolean", "void", "octet", "short", "long", "long",
        "unsigned", "float", "double", "char", "string", "wchar", "wstring",
        "nsid", "domstring", "utf8string", "cstring", "astring", "jsval",
        "uuid", "scriptable", "builtinclass", "function", "noscript", "deprecated",
        "object", "main_process_scriptable_only",
    ];

    let reserved_words_webidl = vec![
        "module", "interface", "partial", "dictionary", "exception", "enum", "callback",
        "typedef", "implements", "const", "null", "true", "false", "serializer",
        "stringifier", "jsonifier", "unrestricted", "attribute", "readonly", "inherit",
        "static", "getter", "setter", "creator", "deleter", "legacycaller", "optional",
        "Date", "DOMString", "ByteString", "USVString", "any", "boolean", "byte",
        "double", "float", "long", "object", "octet", "Promise", "required", "sequence",
        "MozMap", "short", "unsigned", "void", "ArrayBuffer", "SharedArrayBuffer", "or",
        "maplike", "setlike", "iterable",
        "Exposed", "ChromeOnly", "ChromeConstructor", "Pref", "Func", "AvailableIn",
        "CheckAnyPermissions", "CheckAllPermissions", "JSImplementation", "HeaderFile",
        "NavigatorProperty", "AvailableIn", "Func", "CheckAnyPermissions", "CheckAllPermissions",
        "Deprecated", "NeedResolve", "OverrideBuiltins", "ChromeOnly", "Unforgeable",
        "UnsafeInPrerendering", "LegacyEventInit", "ProbablyShortLivingObject", "ArrayClass",
        "Clamp", "Constructor", "EnforceRange", "ExceptionClass", "Exposed", "ImplicitThis",
        "Global", "PrimaryGlobal", "LegacyArrayClass", "LegacyUnenumerableNamedProperties",
        "LenientSetter", "LenientThis", "NamedConstructor", "NewObject", "NoInterfaceObject",
        "OverrideBuiltins", "PutForwards", "Replaceable", "SameObject", "SecureContext",
        "Throws", "TreatNonObjectAsNull", "TreatNullAs", "Unforgeable", "Unscopable",
    ];

    let reserved_words_python = vec![
        "and", "del", "from", "not", "while",
        "as", "elif", "global", "or", "with",
        "assert", "else", "if", "pass", "yield",
        "break", "except", "import", "print",
        "class", "exec", "in", "raise", "continue",
        "finally", "is", "return",
        "def", "for", "lambda", "try",
    ];

    fn make_reserved(v: Vec<&str>) -> HashMap<String, String> {
        let mut reserved_words = HashMap::new();
        for word in v {
            reserved_words.insert(word.to_string(), "style=\"color: blue;\" ".to_string());
        }
        reserved_words
    }

    let js_spec = tokenize::LanguageSpec {
        reserved_words: make_reserved(reserved_words_js),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: true,
        regexp_literals: true,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    let cpp_spec = tokenize::LanguageSpec {
        reserved_words: make_reserved(reserved_words_cpp),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: true,
    };

    let ipdl_spec = tokenize::LanguageSpec {
        reserved_words: make_reserved(reserved_words_ipdl),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    let idl_spec = tokenize::LanguageSpec {
        reserved_words: make_reserved(reserved_words_idl),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    let webidl_spec = tokenize::LanguageSpec {
        reserved_words: make_reserved(reserved_words_webidl),
        hash_comment: false,
        c_style_comments: true,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: false,
        c_preprocessor: false,
    };

    let python_spec = tokenize::LanguageSpec {
        reserved_words: make_reserved(reserved_words_python),
        hash_comment: true,
        c_style_comments: false,
        backtick_strings: false,
        regexp_literals: false,
        triple_quote_literals: true,
        c_preprocessor: false,
    };

    for path in fname_args {
        println!("File {}", path);

        let ext = match Path::new(path).extension() {
            Some(ext) => ext.to_str().unwrap(),
            None => "",
        };
        let lang = match ext {
            "c" | "cc" | "cpp" | "h" | "hh" => ShowAs::Indexed(&cpp_spec),
            "ipdl" | "ipdlh" => ShowAs::Indexed(&ipdl_spec),
            "idl" => ShowAs::Indexed(&idl_spec),
            "webidl" => ShowAs::Indexed(&webidl_spec),
            "js" | "jsm" | "json" => ShowAs::Indexed(&js_spec),
            "py" | "build" => ShowAs::Indexed(&python_spec),

            "ogg" | "ttf" | "xpi" | "png" | "bcmap" |
            "gif" | "ogv" | "jpg" | "bmp" | "icns" | "ico" |
            "mp4" | "sqlite" | "jar" | "webm" | "woff" |
            "class" | "m4s" | "mgif" | "wav" | "opus" |
            "mp3" | "otf" => ShowAs::Binary,

            _ => ShowAs::Plain,
        };

        let output_fname = format!("{}/file/{}", index_root, path);
        let output_file = File::create(output_fname).unwrap();
        let mut writer = BufWriter::new(output_file);

        match lang {
            ShowAs::Binary => {
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

        let tokens = match lang {
            ShowAs::Binary => panic!("Unexpected binary file"),
            ShowAs::Plain => {
                let lines = input.split('\n');
                let mut tokens = Vec::new();
                let mut start = 0;
                for line in lines {
                    if line.len() > 0 {
                        tokens.push(tokenize::Token {
                            start: start,
                            end: start + line.len(),
                            kind: tokenize::TokenKind::PlainText,
                        });
                    }
                    start += line.len();
                    if start == input.len() {
                        break;
                    }
                    tokens.push(tokenize::Token {
                        start: start,
                        end: start + 1,
                        kind: tokenize::TokenKind::Newline,
                    });
                    start += 1;
                }
                tokens
            },
            ShowAs::Indexed(spec) => tokenize::tokenize_c_like(&input, spec),
        };

        let mut output = String::new();
        let mut last = 0;

        let mut line_start = 0;
        let mut cur_line = 1;

        let mut cur_datum = 0;

        fn entity_replace(s: String) -> String {
            s.replace("&", "&amp;").replace("<", "&lt;")
        }

        output.push_str(&format!("<code id=\"line-{}\" aria-labelledby=\"{}\">", cur_line, cur_line));

        let mut generated_json = json::Array::new();

        for token in tokens {
            match token.kind {
                tokenize::TokenKind::Newline => {
                    output.push_str(&input[last .. token.start]);
                    output.push_str("\n</code>");

                    cur_line += 1;
                    line_start = token.end;
                    last = token.end;
                    output.push_str(&format!("<code id=\"line-{}\" aria-labelledby=\"{}\">", cur_line, cur_line));

                    continue;
                },
                _ => {}
            }

            let column = (token.start - line_start) as u32;

            // This should never happen, but sometimes the analysis
            // has bugs in it. This works around them.
            while cur_datum < analysis.len() &&
                cur_line as u32 > analysis[cur_datum].loc.lineno
            {
                cur_datum += 1
            }

            let datum =
                if cur_datum < analysis.len() &&
                cur_line as u32 == analysis[cur_datum].loc.lineno &&
                column == analysis[cur_datum].loc.col_start
            {
                let r = &analysis[cur_datum].data;
                cur_datum += 1;
                Some(r)
            } else {
                None
            };

            let data = match (&token.kind, datum) {
                (&tokenize::TokenKind::Identifier(None), Some(d)) => {
                    let ref id = d[0].sym;

                    let mut menu_jumps : HashMap<String, Json> = HashMap::new();
                    for item in d {
                        let syms = item.sym.split(',');
                        for sym in syms {
                            match jumps.get(sym) {
                                Some(jump) => {
                                    if !(&jump.path == path && jump.lineno == cur_line) {
                                        let key = format!("{}:{}", jump.path, jump.lineno);
                                        let mut obj = json::Object::new();
                                        obj.insert("sym".to_string(), Json::String(sym.to_string()));
                                        obj.insert("pretty".to_string(), Json::String(jump.pretty.clone()));
                                        menu_jumps.insert(key, Json::Object(obj));
                                    }
                                },
                                None => {}
                            }
                        }
                    }

                    let items = d.iter().map(|item| {
                        let mut obj = json::Object::new();
                        obj.insert("pretty".to_string(), Json::String(item.pretty.clone()));
                        obj.insert("sym".to_string(), Json::String(item.sym.clone()));
                        Json::Object(obj)
                    }).collect::<Vec<_>>();

                    let menu_jumps = menu_jumps.into_iter().map(|(_, v)| v).collect::<Vec<_>>();

                    let index = generated_json.len();
                    generated_json.push(Json::Array(vec![Json::Array(menu_jumps), Json::Array(items)]));

                    format!("data-id=\"{}\" data-i=\"{}\" ", id, index)
                },
                _ => "".to_string()
            };

            let style = match token.kind {
                tokenize::TokenKind::Identifier(None) => {
                    match datum {
                        Some(d) => {
                            let styles = d.iter().flat_map(|a| a.syntax.iter().flat_map(|s| match s.as_ref() {
                                "type" => vec!["color: teal;"],
                                "def" | "decl" => vec!["font-weight: 600;"],
                                _ => vec![],
                            }));
                            let styles = styles.collect::<Vec<_>>();
                            if styles.len() > 0 {
                                format!("style=\"{}\" ", styles.join(" "))
                            } else {
                                "".to_owned()
                            }
                        },
                        None => "".to_owned()
                    }
                },
                tokenize::TokenKind::Identifier(Some(ref style)) => style.clone(),
                tokenize::TokenKind::StringLiteral => "style=\"color: green;\" ".to_owned(),
                tokenize::TokenKind::Comment => "style=\"color: darkred;\" ".to_owned(),
                tokenize::TokenKind::RegularExpressionLiteral => "style=\"color: #6d7b8d;\" ".to_owned(),
                _ => "".to_owned()
            };

            match token.kind {
                tokenize::TokenKind::Punctuation => {
                    output.push_str(&input[last .. token.start]);
                    output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                    last = token.end;
                },
                _ => {
                    if style != "" || data != "" {
                        output.push_str(&input[last .. token.start]);
                        output.push_str(&format!("<span {}{}>", style, data));
                        output.push_str(&entity_replace(input[token.start .. token.end].to_string()));
                        output.push_str("</span>");
                        last = token.end;
                    }
                }
            }
        }

        output.push_str(&input[last ..]);
        output.push_str("</code>\n");

        let output = output.replace("\r", "\u{240D}"); // U+240D = CR symbol.

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

        for i in 0 .. cur_line {
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

        write!(writer, "<script>var ANALYSIS_DATA = {};</script>\n",
               json::encode(&Json::Array(generated_json)).unwrap()).unwrap();

        generate_footer(&opt, &mut writer).unwrap();
    }
}
