/**
 * Common rust HTML output logic.  Note that any changes in this file potentially require changes
 * to `scripts/output.js` which includes hard-coded HTML that needs to logically be equivalent to
 * what's in this file.
 **/
use std::io::Write;
use std::path::Path;

extern crate chrono;
use crate::file_format::analysis_manglings::make_file_sym_from_path;

use self::chrono::{DateTime, Local};

pub struct Options<'a> {
    pub title: &'a str,
    pub tree_name: &'a str,
    pub revision: Option<(&'a str, &'a str)>,
    pub include_date: bool,
    /// Extra classes to include on the content element.  This allows less padding to be used on
    /// source listings where we have particular styling needs for "position: sticky" but want
    /// every other display to have normal padding.
    pub extra_content_classes: &'a str,
}

pub fn choose_icon(path: &str) -> String {
    let ext: &str = match Path::new(path).extension() {
        Some(ext) => ext.to_str().unwrap(),
        None => "",
    };
    if ext == "jsm" {
        return "js".to_string();
    }
    if ["cpp", "h", "c", "js", "py"]
        .iter()
        .any(|x: &&str| *x == ext)
    {
        return ext.to_string();
    }
    "".to_string()
}

pub fn file_url(opt: &Options, path: &str) -> String {
    format!("/{}/source/{}", opt.tree_name, path)
}

pub fn generate_breadcrumbs(
    opt: &Options,
    writer: &mut dyn Write,
    path: &str,
    generate_symbol: bool,
) -> Result<(), &'static str> {
    let mut breadcrumbs = format!("<a href=\"{}\">{}</a>", file_url(opt, ""), opt.tree_name);

    breadcrumbs.push_str(r#"<button id="tree-switcher" title="Open tree switcher menu" aria-expanded="false" aria-haspopup="true" aria-controls="tree-switcher-menu"></button>"#);
    breadcrumbs.push_str(r#"<div id="tree-switcher-menu" role="menu" class="context-menu" style="display: none"></div>"#);

    let mut path_so_far = "".to_string();

    if !path.is_empty() {
        for name in path.split('/') {
            breadcrumbs.push_str("<span class=\"path-separator\">/</span>");
            path_so_far.push_str(name);
            breadcrumbs.push_str(&format!(
                "<a href=\"{}\">{}</a>",
                file_url(opt, &path_so_far),
                name
            ));
            path_so_far.push('/');
        }
    }

    if generate_symbol {
        breadcrumbs.push_str(&format!(
            "  <span data-symbols=\"{}\">(file symbol)</span>",
            make_file_sym_from_path(path)
        ));
    }

    write!(
        *writer,
        "<div class=\"breadcrumbs\">{}</div>\n",
        breadcrumbs
    )
    .map_err(|_| "Write err")?;

    Ok(())
}

/// `generate_formatted` input type that allows for hierarchical indentation and
/// not having to call to_string() on everything.
#[derive(Clone)]
pub enum F {
    /// Indents its children by one 2-spaced level.
    /// Use like `F::Indent(vec![...])`.
    Indent(Vec<F>),
    /// Doesn't indent its children.
    /// Use like `F::Seq(vec![...])`.
    Seq(Vec<F>),
    /// For when you don't have a 'static lifetime string literal that's part of
    /// the program source.  Frequently this is the result of a `format!` call.
    /// Use like `F::T(format!(r#"<h>{}</h>"#, some_var))`.
    T(String),
    /// For string literals in the program.  Avoid having to type `to_string()`!
    /// Use like `F::S("<div>")` or `F::S(r#"<a href="/look-quotes">foo</a>"#)`.
    S(&'static str),
}

pub fn generate_formatted(
    writer: &mut dyn Write,
    formatted: &F,
    indent: u32,
) -> Result<(), &'static str> {
    match *formatted {
        F::Indent(ref seq) => {
            for f in seq {
                generate_formatted(writer, &f, indent + 1)?;
            }
            Ok(())
        }
        F::Seq(ref seq) => {
            for f in seq {
                generate_formatted(writer, &f, indent)?;
            }
            Ok(())
        }
        F::T(ref text) => {
            for _ in 0..indent {
                write!(writer, "  ").map_err(|_| "Write err")?;
            }
            write!(writer, "{}\n", text).map_err(|_| "Write err")?;
            Ok(())
        }
        F::S(text) => {
            for _ in 0..indent {
                write!(writer, "  ").map_err(|_| "Write err")?;
            }
            write!(writer, "{}\n", text).map_err(|_| "Write err")?;
            Ok(())
        }
    }
}

pub fn generate_header(opt: &Options, writer: &mut dyn Write) -> Result<(), &'static str> {
    let css = ["mozsearch.css", "icons.css", "font-icons.css"];
    let css_tags = css.iter().map(|c| {
        F::T(format!(
            r#"<link href="/{}/static/css/{}" rel="stylesheet" media="screen"/>"#,
            opt.tree_name, c
        ))
    });

    let mut head_seq = vec![
        F::S(r#"<meta charset="utf-8" />"#),
        F::S(r#"<meta name="color-scheme" content="light dark">"#),
        F::T(format!(
            r#"<link href="/{}/static/icons/search.png" rel="shortcut icon">"#,
            opt.tree_name
        )),
        F::T(format!("<title>{}</title>", opt.title)),
    ];
    head_seq.extend(css_tags);

    let fieldset = vec![
        F::S(r#"<div id="query-section">"#),
        F::Indent(vec![
            F::S(r#"<label for="query" class="query_label visually-hidden">Find</label>"#),
            F::T(format!(
                r#"<input type="text" name="q" value="" maxlength="2048" id="query" accesskey="s" title="Search" placeholder="Search {}" autocomplete="off" />"#,
                opt.tree_name
            )),
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
            F::Indent(vec![F::S(
                r#"<input type="checkbox" name="case" id="case" class="option-checkbox" value="true" accesskey="c"/><span class="access-key">C</span>ase-sensitive"#,
            )]),
            F::S("</label>"),
            F::S(r#"<label for="regexp">"#),
            F::Indent(vec![F::S(
                r#"<input type="checkbox" name="regexp" id="regexp" class="option-checkbox" value="true" accesskey="r"/><span class="access-key">R</span>egexp search"#,
            )]),
            F::S("</label>"),
        ]),
        F::S("</div>"),
        F::S(r#"<div id="path-section">"#),
        F::Indent(vec![
            F::S(r#"<label for="path" class="query_label visually-hidden">Path</label>"#),
            F::S(
                r#"<input type="text" name="path" value="" maxlength="2048" id="path" accesskey="p" title="Path" placeholder="Path filter (supports globbing and ^, $)" autocomplete="off" />"#,
            ),
            F::S(r#"<div class="zero-size-container">"#),
            F::Indent(vec![
                F::S(r#"<div class="bubble" id="path-bubble">"#),
                F::S("</div>"),
            ]),
            F::S("</div>"),
        ]),
        F::S("</div>"),
    ];

    let revision = match opt.revision {
        Some((rev_id, rev_desc)) => vec![
            F::T(format!(
                r#"<span id="rev-id">Showing <a href="/{}/commit/{}">{}</a>:</span>"#,
                opt.tree_name,
                rev_id,
                &rev_id[..8]
            )),
            F::T(format!(r#"<span id="rev-desc">{}</span>"#, rev_desc)),
        ],
        None => vec![],
    };

    let form = vec![
        F::S("<fieldset>"),
        F::Indent(fieldset),
        F::S("</fieldset>"),
        F::S(
            "<!-- disabled to avoid enter-submits behavior that conflicts with JS search logic -->",
        ),
        F::S(r#"<input type="submit" value="Search" disabled class="visually-hidden" />"#),
        F::S(r#"<div id="revision">"#),
        F::Indent(revision),
        F::S("</div>"),
    ];

    let root_class = if opt.revision.is_some() {
        "old-rev"
    } else {
        ""
    };

    let f = F::Seq(vec![
        F::S("<!DOCTYPE html>"),
        F::T(format!(r#"<html lang="en-US" class="{}">"#, root_class)),
        F::S("<head>"),
        F::Indent(head_seq),
        F::S("</head>"),
        F::S(""),
        F::S("<body>"),
        F::Indent(vec![
            F::S(r#"<div id="fixed-header">"#),
            F::T(format!(
                r#"<form method="get" action="/{}/search" class="search-box" id="search-box">"#,
                opt.tree_name
            )),
            F::Indent(form),
            F::S("</form>"),
            F::S("</div>"),
            F::S(r#"<div id="scrolling">"#),
            F::T(format!(
                r#"<div id="content" class="content {}" data-no-results="No results for current query.">"#,
                opt.extra_content_classes
            )),
        ]),
    ]);

    generate_formatted(writer, &f, 0)?;

    Ok(())
}

pub fn generate_footer(
    opt: &Options,
    tree_name: &str,
    path: &str,
    writer: &mut dyn Write,
) -> Result<(), &'static str> {
    let mut date = F::Seq(vec![]);
    if opt.include_date {
        let local: DateTime<Local> = Local::now();
        let time_str = local.to_rfc2822();

        date = F::Seq(vec![
            F::S(r#"<div id="foot" class="footer">"#),
            F::Indent(vec![F::T(format!(
                r#"This page was generated by Searchfox <span class="pretty-date" data-datetime="{}"></span>."#,
                time_str
            ))]),
            F::S("</div>"),
        ]);
    }

    let scripts = [
        "settings.js",
        "search.js",
        "context-menu.js",
        "panel.js",
        "code-highlighter.js",
        "blame.js",
    ];
    let script_tags: Vec<_> = scripts
        .iter()
        .map(|s| {
            F::T(format!(
                r#"<script src="/{}/static/js/{}"></script>"#,
                opt.tree_name, s
            ))
        })
        .collect();

    let f = F::Seq(vec![
        F::Indent(vec![
            F::Indent(vec![F::S("</div>")]),
            date,
            F::T(format!(
                r#"<span id="data" data-root="/" data-search="/{}/search" data-tree="{}" data-path="{}"></span>"#,
                tree_name, tree_name, path
            )),
            F::Seq(script_tags),
            F::S("</div>"), // close out #scrolling
            F::S("</body>"),
        ]),
        F::S("</html>"),
    ]);

    generate_formatted(writer, &f, 0)?;

    Ok(())
}

pub struct PanelItem {
    pub title: String,
    pub link: String,
    /// This is a pattern which will be appended to the URL, where `{}` is
    /// replaced by the line number.
    pub update_link_lineno: &'static str,
    pub accel_key: Option<char>,
    pub copyable: bool,
}

pub struct PanelSection {
    pub name: String,
    pub items: Vec<PanelItem>,
    pub raw_items: Vec<String>,
}

static COPY_ICONS: &str =
    r#"<span class="icon-docs copy-icon"></span><span class="icon-ok tick-icon"></span>"#;

/// Generate HTML for a panel containing the given sections and write it to the
/// provided writer.  This is expected to be called once per document.
pub fn generate_panel(
    opt: &Options,
    writer: &mut dyn Write,
    sections: &[PanelSection],
    collapsed: bool,
) -> Result<(), &'static str> {
    let sections = sections
        .iter()
        .map(|section| {
            let items = section
                .items
                .iter()
                .map(|item| {
                    let update_attr = if !item.update_link_lineno.is_empty() {
                        format!(
                            r#" data-update-link="{}" data-link="{}""#,
                            item.update_link_lineno, item.link
                        )
                    } else {
                        String::new()
                    };
                    let accel = if let Some(key) = item.accel_key {
                        format!(r#" <span class="accel">{}</span>"#, key)
                    } else {
                        String::new()
                    };
                    let is_link = !item.link.is_empty();
                    let copy = if item.copyable {
                        if is_link {
                            format!(
                                r#"<button class="copy" title="Copy to clipboard">{}</button>"#,
                                COPY_ICONS
                            )
                        } else {
                            format!(r#"<span class="icon copy indicator">{}</span>"#, COPY_ICONS)
                        }
                    } else {
                        String::new()
                    };
                    let tag = if is_link { "a" } else { "button" };
                    let href = if is_link {
                        format!(r#" href="{}""#, item.link)
                    } else {
                        String::new()
                    };
                    F::Seq(vec![
                        F::S("<li>"),
                        F::T(format!(
                            r#"<{}{} title="{}" class="icon item"{}>{}{}{}</{}>"#,
                            tag, href, item.title, update_attr, item.title, accel, copy, tag
                        )),
                        F::S("</li>"),
                    ])
                })
                .collect::<Vec<_>>();

            let raw_items = section
                .raw_items
                .iter()
                .map(|raw_item| F::T(raw_item.to_string()))
                .collect::<Vec<_>>();

            F::Seq(vec![
                F::T(format!("<h4>{}</h4>", section.name)),
                F::S("<ul>"),
                F::Seq(items),
                F::Seq(raw_items),
                F::S("</ul>"),
            ])
        })
        .collect::<Vec<_>>();

    let f = F::Seq(vec![
        F::S(r#"<div class="panel" id="panel">"#),
        F::Indent(vec![
            F::S(r#"<button id="panel-toggle">"#),
            F::Indent(vec![
                F::T(format!(
                    r#"<span class="navpanel-icon icon-down-dir{}" aria-hidden="false"></span>"#,
                    if collapsed {
                        ""
                    } else {
                        " expanded"
                    })),
                F::S("Navigation"),
                F::T(format!(
                    r#"<a id="show-settings" title="Go to settings page" href="/{}/pages/settings.html"><span class="navpanel-icon icon-cog expanded" aria-hidden="false"></span></a>"#,
                    opt.tree_name
                )),
            ]),
            F::S("</button>"),
            F::T(format!(
                r#"<section id="panel-content" aria-expanded="{}" aria-hidden="{}"{}>"#,
                if collapsed {
                    "false"
                } else {
                    "true"
                },
                if collapsed {
                    "true"
                } else {
                    "false"
                },
                if collapsed {
                    r#" style="display: none""#
                } else {
                    ""
                })),
            F::S(
                r#"<label class="panel-accel"><input type="checkbox" id="panel-accel-enable" checked="checked">Enable keyboard shortcuts</label>"#,
            ),
            F::Seq(sections),
            F::S("</section>"),
        ]),
        F::S("</div>"),
    ]);

    generate_formatted(writer, &f, 0)?;

    Ok(())
}

pub fn generate_svg_preview(writer: &mut dyn Write, url: &str) -> Result<(), &'static str> {
    let f = F::Seq(vec![
        F::S(r#"<div class="svg-preview">"#),
        F::Indent(vec![
            F::S("<h4>SVG Preview (Scaled)</h4>"),
            F::S(r#"<input type="checkbox" id="svg-preview-checkerboard"/>"#),
            F::S(r#"<label for="svg-preview-checkerboard">Checkerboard</label>"#),
            F::T(format!(r#"<a href="{}">"#, url)),
            F::Indent(vec![F::T(format!(
                r#"<img src="{0}" alt="Preview of {0}"/>"#,
                url
            ))]),
            F::S("</a>"),
        ]),
        F::S("</div>"),
    ]);

    generate_formatted(writer, &f, 0)?;
    Ok(())
}
