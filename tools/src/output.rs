/**
 * Common rust HTML output logic.  Note that any changes in this file potentially require changes
 * to `scripts/output.js` which includes hard-coded HTML that needs to logically be equivalent to
 * what's in this file.
 **/
use std::io::Write;
use std::path::Path;

extern crate chrono;
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
) -> Result<(), &'static str> {
    let mut breadcrumbs = format!("<a href=\"{}\">{}</a>", file_url(opt, ""), opt.tree_name);

    let mut path_so_far = "".to_string();
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
    let css = ["mozsearch.css", "icons.css"];
    let css_tags = css.iter().map(|c| {
        F::T(format!(
            r#"<link href="/static/css/{}" rel="stylesheet" media="screen"/>"#,
            c
        ))
    });

    let mut head_seq = vec![
        F::S(r#"<meta charset="utf-8" />"#),
        F::S(r#"<meta name="color-scheme" content="light dark">"#),
        F::S(r#"<link href="/static/icons/search.png" rel="shortcut icon">"#),
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
        F::S("<!-- disabled to avoid enter-submits behavior that conflicts with JS search logic -->"),
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
        "search.js",
        "context-menu.js",
        "panel.js",
        "code-highlighter.js",
        "blame.js",
    ];
    let script_tags: Vec<_> = scripts
        .iter()
        .map(|s| F::T(format!(r#"<script src="/static/js/{}"></script>"#, s)))
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
}

static COPY_ICONS: &str = r##"
<!-- This Source Code Form is subject to the terms of the Mozilla Public
   - License, v. 2.0. If a copy of the MPL was not distributed with this
   - file, You can obtain one at http://mozilla.org/MPL/2.0/.

   Taken from mozilla-central@fc6d20cf008c2ca66d2c45439690d2ff067bf1b9/browser/extensions/screenshots/icons/copy.svg
   -->
<svg viewbox="0 0 16 16" class="copy-icon">
  <path d="M14.707 8.293l-3-3A1 1 0 0 0 11 5h-1V4a1 1 0 0 0-.293-.707l-3-3A1 1 0 0 0 6 0H3a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3v3a2 2 0 0 0 2 2h5a2 2 0 0 0 2-2V9a1 1 0 0 0-.293-.707zM12.586 9H11V7.414zm-5-5H6V2.414zM6 7v2H3V2h2v2.5a.5.5 0 0 0 .5.5H8a2 2 0 0 0-2 2zm2 7V7h2v2.5a.5.5 0 0 0 .5.5H13v4z"/>
</svg>

<!-- This Source Code Form is subject to the terms of the Mozilla Public
   - License, v. 2.0. If a copy of the MPL was not distributed with this
   - file, You can obtain one at http://mozilla.org/MPL/2.0/.

  Taken from https://searchfox.org/mozilla-central/rev/819be4899a92213abf121b449779ced662f2ce13/widget/nsNativeBasicTheme.cpp#226-230 with coordinates converted appropriately.
-->
<svg viewbox="0 0 16 16" class="tick-icon">
  <path d="M 2.857143 8.571428 L 6.285714 12.571428 L 7.428571 12.571428 L 13.714286 5.142857 L 13.428572 3.428571 L 12 3.428571 L 7.428571 9.142858 L 6.285714 9.428572 L 4 6.857143"/>
</svg>
"##;

/// Generate HTML for a panel containing the given sections and write it to the
/// provided writer.  This is expected to be called once per document.
pub fn generate_panel(
    writer: &mut dyn Write,
    sections: &[PanelSection],
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
                                r#"<button class="icon copy" title="Copy to clipboard">{}</button>"#,
                                COPY_ICONS
                            )
                        } else {
                            format!(
                                r#"<span class="icon copy indicator">{}</span>"#,
                                COPY_ICONS
                            )
                        }
                    } else {
                        String::new()
                    };
                    let tag = if is_link {
                        "a"
                    } else {
                        "button"
                    };
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

            F::Seq(vec![
                F::T(format!("<h4>{}</h4>", section.name)),
                F::S("<ul>"),
                F::Seq(items),
                F::S("</ul>"),
            ])
        })
        .collect::<Vec<_>>();

    let f = F::Seq(vec![
        F::S(r#"<div class="panel" id="panel">"#),
        F::Indent(vec![
            F::S(r#"<button id="panel-toggle">"#),
            F::Indent(vec![
                F::S(r#"<span class="navpanel-icon expanded" aria-hidden="false"></span>"#),
                F::S("Navigation"),
            ]),
            F::S("</button>"),
            F::S(r#"<section id="panel-content" aria-expanded="true" aria-hidden="false">"#),
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

/// InfoBoxes live in the scrolling content area above the bread-crumbs and
/// house contextual information about the file that is also (possibly)
/// presented in the file listing.  In the future, InfoBoxes may also be emitted
/// in directory listings.
///
/// The intent is to provide ambient information about files like:
/// - Hey, this test file you're looking at is disabled and never runs.
/// - Hey, this test file you're looking at fails intermittently and here are
///   links to the bugs on it!
///
/// And directory listings might contain content like:
/// - There's a README file for this directory but you'll have to scroll down
///   to see it, or click here!  (For cases where the directory listing is known
///   to be long relative to page size.  Could involve hip media queries/dynamic
///   CSS if instantaneous.)
/// - Hey, a bunch of test files in this directory are disabled!
///
pub struct InfoBox {
    /// The heading / name of the info box that will be wrapped by an <h> tag
    /// without escaping.
    pub heading_html: String,
    /// Infobox contents that will be placed in a <div> or similar without
    /// escaping.
    pub body_nodes: Vec<F>,
    pub box_kind: String,
}

/// Generate HTML for the provided info-boxes and write it to the provided
/// writer.  This is expected to be called once per document.
pub fn generate_info_boxes(
    writer: &mut dyn Write,
    info_boxes: &[InfoBox],
) -> Result<(), &'static str> {
    let info_boxes = info_boxes
        .into_iter()
        .map(|info_box| {
            F::Seq(vec![
                F::T(format!(
                    r#"<section class="info-box info-box-{}">"#,
                    info_box.box_kind
                )),
                F::Indent(vec![
                    F::T(format!("<h4>{}</h4>", info_box.heading_html)),
                    F::S("<div>"),
                    F::Indent(info_box.body_nodes.clone()),
                    F::S("</div>"),
                ]),
                F::S("</div>"),
            ])
        })
        .collect::<Vec<_>>();

    let f = F::Seq(vec![
        F::S(r#"<section class="info-boxes" id="info-boxes-container">"#),
        F::Indent(vec![F::Seq(info_boxes)]),
        F::S("</section>"),
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
