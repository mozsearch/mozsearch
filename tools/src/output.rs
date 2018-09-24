use std::io::Write;
use std::path::Path;

extern crate chrono;
use self::chrono::{DateTime, Local};

pub struct Options<'a> {
    pub title: &'a str,
    pub tree_name: &'a str,
    pub revision: Option<(&'a str, &'a str)>,
    pub include_date: bool,
}

pub fn choose_icon(path: &str) -> String {
    let ext : &str = match Path::new(path).extension() {
        Some(ext) => ext.to_str().unwrap(),
        None => "",
    };
    if ext == "jsm" {
        return "js".to_string();
    }
    if ["cpp", "h", "c", "js", "py"].iter().any(|x: &&str| *x == ext) {
        return ext.to_string();
    }
    "".to_string()
}

pub fn file_url(opt: &Options, path: &str) -> String {
    format!("/{}/source/{}", opt.tree_name, path)
}

pub fn generate_breadcrumbs(opt: &Options, writer: &mut Write, path: &str) -> Result<(), &'static str>
{
    let mut breadcrumbs = format!("<a href=\"{}\">{}</a>", file_url(opt, ""), opt.tree_name);

    let mut path_so_far = "".to_string();
    for name in path.split('/') {
        breadcrumbs.push_str("<span class=\"path-separator\">/</span>");
        path_so_far.push_str(name);
        breadcrumbs.push_str(&format!("<a href=\"{}\">{}</a>", file_url(opt, &path_so_far), name));
        path_so_far.push('/');
    }

    try!(write!(*writer, "<div class=\"breadcrumbs\">{}</div>\n", breadcrumbs).map_err(|_| "Write err"));

    Ok(())
}

pub enum F {
    Indent(Vec<F>),
    Seq(Vec<F>),
    T(String),
    S(&'static str),
}

pub fn generate_formatted(writer: &mut Write, formatted: &F, indent: u32) -> Result<(), &'static str>
{
    match *formatted {
        F::Indent(ref seq) => {
            for f in seq {
                try!(generate_formatted(writer, &f, indent + 1));
            }
            Ok(())
        },
        F::Seq(ref seq) => {
            for f in seq {
                try!(generate_formatted(writer, &f, indent));
            }
            Ok(())
        },
        F::T(ref text) => {
            for _ in 0 .. indent {
                try!(write!(writer, "  ").map_err(|_| "Write err"));
            }
            try!(write!(writer, "{}\n", text).map_err(|_| "Write err"));
            Ok(())
        },
        F::S(text) => {
            for _ in 0 .. indent {
                try!(write!(writer, "  ").map_err(|_| "Write err"));
            }
            try!(write!(writer, "{}\n", text).map_err(|_| "Write err"));
            Ok(())
        },
    }
}

pub fn generate_header(opt: &Options, writer: &mut Write) -> Result<(), &'static str>
{
    let css = [
        "mozsearch.css",
        "icons.css",
        "selector-common.css",
        "filter.css",
        "xcode.css",
    ];
    let css_tags =
        css.iter().map(|c| {
            F::T(format!(r#"<link href="/static/css/{}" rel="stylesheet" type="text/css" media="screen"/>"#, c))
        });

    let mut head_seq = vec![
        F::S(r#"<meta charset="utf-8" />"#),
        F::S(r#"<link href="/static/icons/search.png" rel="shortcut icon">"#),
        F::T(format!("<title>{}</title>", opt.title)),
    ];
    head_seq.extend(css_tags);

    let fieldset = vec![
        F::S(r#"<div id="search-box" class="h-flex-container" role="group">"#),
        F::Indent(vec![
            F::S(r#"<div id="query-section">"#),
            F::Indent(vec![
                F::S(r#"<label for="query" class="query_label visually-hidden">Find</label>"#),
                F::T(format!(r#"<input type="text" name="q" value="" maxlength="2048" id="query" accesskey="s" title="Search" placeholder="Search {}" autocomplete="off" />"#,
                             opt.tree_name)),
                F::S(r#"<div class="zero-size-container">"#),
                F::Indent(vec![
                    F::S(r#"<div class="bubble" id="query-bubble">"#),
                    F::S("</div>"),
                ]),
                F::S("</div>"),
                F::S(r#"<section id="spinner"></section>"#),
            ]),
            F::S("</div>"),

            F::S(r#"<div id="option-section" class="v-flex-container">"#),
            F::Indent(vec![
                F::S(r#"<label for="case">"#),
                F::Indent(vec![
                    F::S(r#"<input type="checkbox" name="case" id="case" class="option-checkbox" value="true" accesskey="c"/><span class="access-key">C</span>ase-sensitive"#),
                ]),
                F::S("</label>"),

                F::S(r#"<label for="regexp">"#),
                F::Indent(vec![
                    F::S(r#"<input type="checkbox" name="regexp" id="regexp" class="option-checkbox" value="true" accesskey="r"/><span class="access-key">R</span>egexp search"#),
                ]),
                F::S("</label>"),
            ]),
            F::S("</div>"),

            F::S(r#"<div id="path-section">"#),
            F::Indent(vec![
                F::S(r#"<label for="query" class="query_label visually-hidden">Path</label>"#),
                F::S(r#"<input type="text" name="path" value="" maxlength="2048" id="path" accesskey="p" title="Path" placeholder="Path filter (supports globbing and ^, $)" autocomplete="off" />"#),
                F::S(r#"<div class="zero-size-container">"#),
                F::Indent(vec![
                    F::S(r#"<div class="bubble" id="path-bubble">"#),
                    F::S("</div>"),
                ]),
                F::S("</div>"),
            ]),
            F::S("</div>"),
        ]),
        F::S("</div>"),
    ];

    let revision = match opt.revision {
        Some((rev_id, rev_desc)) => vec![
            F::T(format!(r#"<span id="rev-id">Showing <a href="/{}/commit/{}">{}</a>:</span>"#,
                         opt.tree_name, rev_id, &rev_id[..8])),
            F::T(format!(r#"<span id="rev-desc">{}</span>"#, rev_desc)),
        ],
        None => vec![],
    };

    let body_class = match opt.revision {
        Some(_) => "old-rev",
        None => "",
    };

    let form = vec![
        F::S("<fieldset>"),
        F::Indent(fieldset),
        F::S("</fieldset>"),
        F::S(r#"<input type="submit" value="Search" class="visually-hidden" />"#),
        F::S(r#"<div id="revision">"#),
        F::Indent(revision),
        F::S("</div>"),
    ];

    let f = F::Seq(vec![
        F::S("<!DOCTYPE html>"),
        F::S(r#"<html lang="en-US">"#),
        F::S("<head>"),
        F::Indent(head_seq),
        F::S("</head>"),
        F::S(""),
        F::T(format!(r#"<body class="{}">"#, body_class)),
        F::Indent(vec![
            F::T(format!(r#"<form method="get" action="/{}/search" id="basic_search" class="search-box">"#,
                         opt.tree_name)),
            F::Indent(form),
            F::S("</form>"),

            F::S(r#"<div id="content" class="content" data-no-results="No results for current query.">"#),
        ]),
    ]);

    try!(generate_formatted(writer, &f, 0));

    Ok(())
}

pub fn generate_footer(opt: &Options, tree_name: &str, path: &str, writer: &mut Write) -> Result<(), &'static str>
{
    let mut date = F::Seq(vec![]);
    if opt.include_date {
        let local: DateTime<Local> = Local::now();
        let time_str = local.to_rfc2822();

        date = F::Seq(vec![
            F::S(r#"<div id="foot" class="footer">"#),
            F::Indent(vec![
                F::T(format!(r#"This page was generated by Searchfox <span class="pretty-date" data-datetime="{}"></span>."#, time_str)),
            ]),
            F::S("</div>"),
        ]);
    }

    let scripts = [
        "libs/jquery-2.1.3.min.js",
        "libs/nunjucks.min.js",
        "utils.js",
        "dxr.js",
        "context-menu.js",
        "filter.js",
        "panel.js",
        "code-highlighter.js",
        "blame.js",
    ];
    let script_tags: Vec<_> =
        scripts.iter().map(|s| F::T(format!(r#"<script src="/static/js/{}"></script>"#, s))).collect();

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![F::S("</div>")]),
            date,
            F::T(format!(r#"<span id="data" data-root="/" data-search="/{}/search" data-tree="{}" data-path="{}"></span>"#,
                         tree_name, tree_name, path)),
            F::Seq(script_tags),
            F::S("</body>"),
        ]),
        F::S("</html>")
    ]);

    try!(generate_formatted(writer, &f, 0));

    Ok(())
}

pub struct PanelItem {
    pub title: String,
    pub link: String,
    pub update_link_lineno: bool,
    pub accel_key: Option<char>,
}

pub struct PanelSection {
    pub name: String,
    pub items: Vec<PanelItem>,
}

pub fn generate_panel(writer: &mut Write, sections: &[PanelSection]) -> Result<(), &'static str> {
    let sections = sections.iter().map(|section| {
        let items = section.items.iter().map(|item| {
            let update_attr = if item.update_link_lineno {
                format!(r#" data-update-link="true" data-link="{}""#, item.link)
            } else {
                "".to_owned()
            };
            let accel = if let Some(key) = item.accel_key {
                format!(r#" <span class="accel">{}</span>"#, key)
            } else {
                "".to_owned()
            };
            F::Seq(vec![
                F::S("<li>"),
                F::T(format!(r#"<a href="{}" title="{}" class="icon"{}>{}{}</a>"#,
                             item.link, item.title, update_attr, item.title, accel)),
                F::S("</li>"),
            ])
        }).collect::<Vec<_>>();

        F::Seq(vec![
            F::T(format!("<h4>{}</h4>", section.name)),
            F::S("<ul>"),
            F::Seq(items),
            F::S("</ul>"),
        ])
    }).collect::<Vec<_>>();

    let f = F::Seq(vec![
        F::S(r#"<div class="panel">"#),
        F::Indent(vec![
            F::S(r#"<button id="panel-toggle">"#),
            F::Indent(vec![
                F::S(r#"<span class="navpanel-icon expanded" aria-hidden="false"></span>"#),
                F::S("Navigation"),
            ]),
            F::S("</button>"),
            F::S(r#"<section id="panel-content" aria-expanded="true" aria-hidden="false">"#),
            F::Seq(sections),
            F::S("</section>"),
        ]),
        F::S("</div>"),
    ]);

    try!(generate_formatted(writer, &f, 0));

    Ok(())
}

pub fn generate_svg_preview(writer: &mut Write, url: &str) -> Result<(), &'static str> {
    let f = F::Seq(vec![
      F::S(r#"<div class="svg-preview">"#),
      F::Indent(vec![
        F::S("<h4>SVG Preview (Scaled)</h4>"),
        F::S(r#"<input type="checkbox" id="svg-preview-checkerboard"/>"#),
        F::S(r#"<label for="svg-preview-checkerboard">Checkerboard</label>"#),
        F::T(format!(r#"<a href="{}">"#, url)),
        F::Indent(vec![
          F::T(format!(r#"<img src="{0}" alt="Preview of {0}"/>"#, url)),
        ]),
        F::S("</a>"),
      ]),
      F::S("</div>")
    ]);

    try!(generate_formatted(writer, &f, 0));
    Ok(())
}
