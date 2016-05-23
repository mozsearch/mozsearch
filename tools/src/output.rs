use std::io::Write;
use std::path::Path;

extern crate chrono;
use self::chrono::{DateTime, Local};

pub struct Options<'a> {
    pub title: &'a str,
    pub tree_name: &'a str,
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
        "forms.css",
        "icons.css",
        "selector-common.css",
        "filter.css",
        "xcode.css",
    ];
    let css_tags =
        css.iter().map(|c| {
            F::T(format!("<link href=\"/static/css/{}\" rel=\"stylesheet\" type=\"text/css\" media=\"screen\"/>", c))
        });

    let mut head_seq = vec![
        F::S("<meta charset=\"utf-8\" />"),
        F::T(format!("<title>{}</title>", opt.title)),
    ];
    head_seq.extend(css_tags);

    let fieldset = vec![
        F::S("<div id=\"search-box\" class=\"flex-container\" role=\"group\">"),
        F::Indent(vec![
            F::S("<div class=\"elem_container find\">"),
            F::Indent(vec![
                F::S("<label for=\"query\" class=\"query_label visually-hidden\">Find</label>"),
                F::T(format!("<input type=\"text\" name=\"q\" value=\"\" maxlength=\"2048\" id=\"query\" class=\"query\" accesskey=\"s\" title=\"Search\" placeholder=\"Search {}\" autocomplete=\"off\" />",
                             opt.tree_name)),
                F::S("<div class=\"zero-size-container\">"),
                F::Indent(vec![
                    F::S("<div class=\"bubble\">"),
                    F::S("</div>"),
                ]),
                F::S("</div>"),

                F::S("<section id=\"search-filter\" class=\"search-filter\">"),
                F::Indent(vec![
                    F::S("<button type=\"button\" class=\"sf-select-trigger\" aria-label=\"Select Filter\">"),
                    F::Indent(vec![
                        F::S("<!-- arrow icon using icon font -->"),
                        F::S("<span aria-hidden=\"true\" data-filter-arrow=\"&#xe801;\" class=\"sf-selector-arrow\">"),
                        F::Indent(vec![F::S("Filters")]),
                        F::S("</span>"),
                    ]),
                    F::S("</button>"),
                ]),
                F::S("</section>"),
                F::S("<div class=\"sf-select-options sf-modal\" aria-expanded=\"false\">"),
                F::Indent(vec![
                    F::S("<ul class=\"selector-options\" tabindex=\"-1\">"),
                    F::S("</ul>"),
                ]),
                F::S("</div>"),
            ]),
            F::S("</div>"),

            F::S("<div class=\"elem_container case\">"),
            F::Indent(vec![
                F::S("<label for=\"case\">"),
                F::Indent(vec![
                    F::S("<input type=\"checkbox\" name=\"case\" id=\"case\" class=\"checkbox_case\" value=\"true\" accesskey=\"c\"/><span class=\"access-key\">C</span>ase-sensitive"),
                ]),
                F::S("</label>"),
            ]),
            F::S("</div>"),
        ]),
        F::S("</div>"),
    ];

    let form = vec![
        F::S("<fieldset>"),
        F::Indent(fieldset),
        F::S("</fieldset>"),
        F::T(format!("<input type=\"hidden\" value=\"{}\" id=\"ts-value\" />", opt.tree_name)),
        F::S("<input type=\"hidden\" name=\"redirect\" value=\"true\" id=\"redirect\" />"),
        F::S("<input type=\"submit\" value=\"Search\" class=\"visually-hidden\" />"),
    ];

    let f = F::Seq(vec![
        F::S("<!DOCTYPE html>"),
        F::S("<html lang=\"en-US\">"),
        F::S("<head>"),
        F::Indent(head_seq),
        F::S("</head>"),
        F::S(""),
        F::S("<body>"),
        F::Indent(vec![
            F::T(format!("<form method=\"get\" action=\"/{}/search\" id=\"basic_search\" class=\"search-box\">",
                         opt.tree_name)),
            F::Indent(form),
            F::S("</form>"),

            F::S("<div id=\"content\" class=\"content\" data-no-results=\"No results for current query.\">"),
        ]),
    ]);

    try!(generate_formatted(writer, &f, 0));

    Ok(())
}

pub fn generate_footer(opt: &Options, writer: &mut Write) -> Result<(), &'static str>
{
    let mut date = F::Seq(vec![]);
    if opt.include_date {
        let local: DateTime<Local> = Local::now();
        let time_str = local.to_rfc2822();

        date = F::Seq(vec![
            F::S("<div id=\"foot\" class=\"footer\">"),
            F::Indent(vec![
                F::T(format!("This page was generated by Searchfox \
                              <span class=\"pretty-date\" data-datetime=\"{}\"></span>.", time_str)),
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
        scripts.iter().map(|s| F::T(format!("<script src=\"/static/js/{}\"></script>", s))).collect();

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![F::S("</div>")]),
            date,
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
}

pub struct PanelSection {
    pub name: String,
    pub items: Vec<PanelItem>,
}

pub fn generate_panel(writer: &mut Write, sections: &Vec<PanelSection>) -> Result<(), &'static str> {
    let sections = sections.iter().map(|section| {
        let items = section.items.iter().map(|item| {
            let update_attr = if item.update_link_lineno {
                format!(" data-update-link=\"true\" data-link=\"{}\"", item.link)
            } else {
                "".to_owned()
            };
            F::Seq(vec![
                F::S("<li>"),
                F::T(format!("<a href=\"{}\" title=\"{}\" class=\"icon\"{}>{}</a>",
                             item.link, item.title, update_attr, item.title)),
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
        F::S("<div class=\"panel\">"),
        F::Indent(vec![
            F::S("<button id=\"panel-toggle\">"),
            F::Indent(vec![
                F::S("<span class=\"navpanel-icon expanded\" aria-hidden=\"false\"></span>"),
                F::S("Navigation"),
            ]),
            F::S("</button>"),
            F::S("<section id=\"panel-content\" aria-expanded=\"true\" aria-hidden=\"false\">"),
            F::Seq(sections),
            F::S("</section>"),
        ]),
        F::S("</div>"),
    ]);

    try!(generate_formatted(writer, &f, 0));

    Ok(())
}
