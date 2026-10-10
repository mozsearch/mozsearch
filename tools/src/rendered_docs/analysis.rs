//! The analysis records of documents, from rendering them: the Sphinx labels
//! they define (`DOCLABEL_` symbols, as doc-defs), and their references to
//! labels and to other documents (`FILE_` symbols, as doc-uses).  The
//! cross-reference of all of the tree's documents is then Sphinx's index of
//! labels, which rendered views use to link to labels in other documents
//! (see `Links::label_url`).

use std::collections::HashSet;

use super::{DocRecord, Links, file_symbol, markdown, rst};
use crate::file_format::analysis::{AnalysisKind, Location, SourceRange};
use crate::file_format::doc_trees::DocTrees;
use crate::semantic_analyzer::{Record, source_record, target_record};

/// The tree's files, which documents link to.
pub struct TreeFiles {
    files: HashSet<String>,
    dirs: HashSet<String>,
    doc_trees: Option<DocTrees>,
}

impl TreeFiles {
    /// The files at `paths`, relative to the root of the tree, and the map of
    /// the documentation's staged layout, if the tree has one.
    pub fn new(paths: impl IntoIterator<Item = String>, doc_trees: Option<DocTrees>) -> TreeFiles {
        let files: HashSet<String> = paths.into_iter().collect();
        let mut dirs = HashSet::new();
        for path in &files {
            let mut path = path.as_str();
            while let Some((dir, _)) = path.rsplit_once('/') {
                if !dirs.insert(dir.to_string()) {
                    break;
                }
                path = dir;
            }
        }
        TreeFiles {
            files,
            dirs,
            doc_trees,
        }
    }
}

/// (Analysis only needs to know which files there are; nothing links.)
impl Links for TreeFiles {
    fn exists(&self, path: &str) -> bool {
        self.files.contains(path) || self.dirs.contains(path)
    }

    fn source_url(&self, _path: &str) -> String {
        String::new()
    }

    fn rendered_url(&self, _path: &str) -> String {
        String::new()
    }

    fn raw_url(&self, _path: &str) -> Option<String> {
        None
    }

    fn search_url(&self, _query: &str) -> String {
        String::new()
    }

    fn tree_name(&self) -> &str {
        ""
    }

    fn staged_doc(&self, staged: &str) -> Option<String> {
        self.doc_trees.as_ref()?.find_source(staged)
    }
}

/// The analysis records of the document at `path` (with `source` contents),
/// but for its file definition: a source and a target record for each label
/// it defines and each reference to a label or a document, whose context is
/// the document (by its title).
pub fn doc_records(path: &str, source: &str, files: &TreeFiles) -> Vec<Record> {
    let rendered = if path.ends_with(".rst") {
        rst::render(source, path, files)
    } else {
        markdown::render(source, path, files)
    };
    let context = rendered.title.clone().unwrap_or_else(|| path.to_string());
    let contextsym = file_symbol(path);
    let mut line_starts = vec![0];
    line_starts.extend(source.match_indices('\n').map(|(i, _)| i + 1));
    let mut records = vec![];
    for DocRecord {
        def,
        sym,
        pretty,
        range,
        syntax,
    } in rendered.records
    {
        // (A range spanning lines ends at the end of its first line.)
        let line = line_starts.partition_point(|&start| start <= range.start) - 1;
        let line_end = line_starts
            .get(line + 1)
            .map_or(source.len(), |&next| next - 1);
        let loc = Location {
            lineno: line as u32 + 1,
            col_start: (range.start - line_starts[line]) as u32,
            col_end: (range.end.min(line_end) - line_starts[line]) as u32,
        };
        let (role, kind) = if def {
            ("def", AnalysisKind::DocDef)
        } else {
            ("use", AnalysisKind::DocUse)
        };
        records.push(source_record(
            loc,
            role,
            format!("{} {}", syntax, pretty),
            sym.clone(),
            SourceRange::default(),
        ));
        records.push(target_record(
            loc,
            kind,
            pretty,
            sym,
            context.clone(),
            contextsym.clone(),
        ));
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(paths: &[&str]) -> TreeFiles {
        TreeFiles::new(paths.iter().map(|p| p.to_string()), None)
    }

    fn lines(records: &[Record]) -> Vec<String> {
        records.iter().map(Record::to_json).collect()
    }

    #[test]
    fn test_markdown_records() {
        let source = "# The Title\n\n(my-label)=\n## More\n\nSee {ref}`the title` and [b](b.md).\n";
        let records = doc_records("docs/a.md", source, &files(&["docs/a.md", "docs/b.md"]));
        assert_eq!(
            lines(&records),
            vec![
                r#"{"loc":"00001:2-11","source":1,"syntax":"def","pretty":"label The Title","sym":"DOCLABEL_the@2Dtitle"}"#,
                r#"{"loc":"00001:2-11","target":1,"kind":"doc-def","pretty":"The Title","sym":"DOCLABEL_the@2Dtitle","context":"The Title","contextsym":"FILE_docs/a@2Emd"}"#,
                r#"{"loc":"00003:1-9","source":1,"syntax":"def","pretty":"label my-label","sym":"DOCLABEL_my@2Dlabel"}"#,
                r#"{"loc":"00003:1-9","target":1,"kind":"doc-def","pretty":"my-label","sym":"DOCLABEL_my@2Dlabel","context":"The Title","contextsym":"FILE_docs/a@2Emd"}"#,
                r#"{"loc":"00006:10-19","source":1,"syntax":"use","pretty":"label the title","sym":"DOCLABEL_the@2Dtitle"}"#,
                r#"{"loc":"00006:10-19","target":1,"kind":"doc-use","pretty":"the title","sym":"DOCLABEL_the@2Dtitle","context":"The Title","contextsym":"FILE_docs/a@2Emd"}"#,
                r#"{"loc":"00006:29-33","source":1,"syntax":"use","pretty":"document docs/b.md","sym":"FILE_docs/b@2Emd"}"#,
                r#"{"loc":"00006:29-33","target":1,"kind":"doc-use","pretty":"docs/b.md","sym":"FILE_docs/b@2Emd","context":"The Title","contextsym":"FILE_docs/a@2Emd"}"#,
            ]
        );
    }

    #[test]
    fn test_rst_records() {
        let source = "\
.. _top-label:

=====
Title
=====

Sub
===

See :ref:`top-label`, `Sub`_, top-label_, :doc:`other`, and :ref:`elsewhere`.

.. toctree::

   other
";
        let records = doc_records("d/a.rst", source, &files(&["d/a.rst", "d/other.rst"]));
        let summary: Vec<String> = records
            .iter()
            .filter_map(|record| match record {
                Record::Target(target) => Some(format!(
                    "{}:{}-{} {} {}",
                    target.loc.lineno,
                    target.loc.col_start,
                    target.loc.col_end,
                    target.data.kind.to_ustr(),
                    target.data.sym
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                "1:4-13 doc-def DOCLABEL_top@2Dlabel",
                "4:0-5 doc-def DOCLABEL_title",
                "10:10-19 doc-use DOCLABEL_top@2Dlabel",
                // (`Sub` is a section title, but not a label.)
                "10:30-39 doc-use DOCLABEL_top@2Dlabel",
                "10:48-53 doc-use FILE_d/other@2Erst",
                "10:66-75 doc-use DOCLABEL_elsewhere",
                "14:3-8 doc-use FILE_d/other@2Erst",
            ]
        );
    }
}
