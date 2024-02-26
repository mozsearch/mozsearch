use std::collections::VecDeque;

use async_trait::async_trait;
use clap::{Args, ValueEnum};
use itertools::Itertools;

use super::{
    interface::{
        BasicMarkup, PipelineCommand, PipelineValues, SymbolTreeTable, SymbolTreeTableCell, SymbolTreeTableList, SymbolTreeTableNode
    },
    symbol_graph::DerivedSymbolInfo,
};

use crate::abstract_server::{AbstractServer, ErrorDetails, ErrorLayer, Result, ServerError};

#[derive(Clone, Debug, PartialEq, ValueEnum)]
pub enum SymbolFormatMode {
    FieldLayout,
    // - class-field-use-matrix: table for each class, look up all its methods and all its
    //   fields, then filter the method "calls" to the fields.
    // - caller-matrix: look up a class, get all its methods.  look up all of
    //   the callers of all of those methods.  group them by their class.
    //   - row depth 0 is subsystem
    //   - row depth 1 is class or file if no class
    //   - row depth 2 is method/function
    //   - columns are the methods on the class, probably alphabetical.
    //     - columns could maybe have an upsell to the arg-matrix?
    //   - cells are a count.
    // - arg-matrix:
    //   - like caller-matrix but only for a single matrix and the columns are
    //     the args.
}

/// Given a list of symbol crossref infos, produce a SymbolTreeTable for display
/// purposes.
#[derive(Debug, Args)]
pub struct FormatSymbols {
    #[clap(long, value_parser, value_enum, default_value = "field-layout")]
    pub mode: SymbolFormatMode,
}

#[derive(Debug)]
pub struct FormatSymbolsCommand {
    pub args: FormatSymbols,
}

#[async_trait]
impl PipelineCommand for FormatSymbolsCommand {
    async fn execute(
        &self,
        server: &Box<dyn AbstractServer + Send + Sync>,
        input: PipelineValues,
    ) -> Result<PipelineValues> {
        let cil = match input {
            PipelineValues::SymbolCrossrefInfoList(cil) => cil,
            _ => {
                return Err(ServerError::StickyProblem(ErrorDetails {
                    layer: ErrorLayer::ConfigLayer,
                    message: "format-symbols needs a CrossrefInfoList".to_string(),
                }));
            }
        };

        match self.args.mode {
            SymbolFormatMode::FieldLayout => {
                let mut tables = vec![];

                for nom_sym_info in cil.symbol_crossref_infos {
                    let mut stt = SymbolTreeTable::new();
                    let (root_sym_id, _) = stt.node_set.add_symbol(DerivedSymbolInfo::new(
                        nom_sym_info.symbol,
                        nom_sym_info.crossref_info,
                        0,
                    ));

                    let mut pending_ids = VecDeque::new();
                    pending_ids.push_back(root_sym_id.clone());

                    let mut root_node = SymbolTreeTableNode {
                        sym_id: Some(root_sym_id),
                        label: vec![],
                        col_vals: vec![],
                        children: vec![],
                    };

                    while let Some(sym_id) = pending_ids.pop_front() {
                        let sym_info = stt.node_set.get(&sym_id);
                        let depth = sym_info.depth;
                        let Some(structured) = sym_info.get_structured() else {
                            continue;
                        };

                        for super_info in &structured.supers {
                            let (super_id, _) = stt
                                .node_set
                                .ensure_symbol(&super_info.sym, server, depth + 1)
                                .await?;
                            pending_ids.push_back(super_id);
                        }

                        let mut class_node = SymbolTreeTableNode {
                            sym_id: Some(sym_id),
                            label: vec![BasicMarkup::Heading(structured.pretty.to_string())],
                            col_vals: vec![],
                            children: vec![],
                        };

                        let platforms_and_fields = structured.fields_across_all_variants();
                        let mut per_plat_fields_by_defloc = vec![];
                        for (platforms, fields) in platforms_and_fields {
                            let plat_idx = class_node.col_vals.len();
                            class_node.col_vals.push(SymbolTreeTableCell::header_text(
                                platforms.join(" ").to_owned(),
                            ));
                            let mut plat_fields_by_defloc = vec![];
                            for field in fields {
                                let (field_id, field_info) = stt
                                    .node_set
                                    .ensure_symbol(&field.sym, server, depth + 1)
                                    .await?;
                                plat_fields_by_defloc.push((
                                    field_info.get_def_lno(),
                                    plat_idx,
                                    field,
                                    field_id,
                                ));
                            }
                            plat_fields_by_defloc.sort_by_key(|ft| ft.0);
                            per_plat_fields_by_defloc.push(plat_fields_by_defloc);
                        }

                        for (_lno, group) in &per_plat_fields_by_defloc
                            .into_iter()
                            // merge in order of lexical line number of the field definition
                            .kmerge_by(|a, b| a.0 < b.0)
                            // and then also group on that line number
                            .group_by(|x| x.0)
                        {
                            let mut field_node = SymbolTreeTableNode {
                                sym_id: None,
                                label: vec![],
                                col_vals: vec![],
                                children: vec![],
                            };
                            // and then within the group let's process in order of increasing platform to match the columns.
                            for (_lno, plat_idx, field_info, field_id) in
                                group.sorted_by_key(|pfi| pfi.1)
                            {
                                if field_node.sym_id.is_none() {
                                    field_node.sym_id = Some(field_id);
                                    field_node.label =
                                        vec![BasicMarkup::Text(match &field_info.type_pretty.is_empty() {
                                            false => format!(
                                                "{} - {}",
                                                field_info.pretty, field_info.type_pretty
                                            ),
                                            true => format!("{}", field_info.pretty),
                                        })];
                                }
                                while field_node.col_vals.len() < plat_idx {
                                    field_node.col_vals.push(SymbolTreeTableCell::empty());
                                }
                                field_node.col_vals.push(SymbolTreeTableCell::text(format!(
                                    "offset {:#x} len {:#x}",
                                    field_info.offset_bytes,
                                    field_info.size_bytes.unwrap_or(0),
                                )));

                                // XXX uh, for at least IDBFactory some weird stuff happens for mRefCnt.
                                if field_node.col_vals.len() >= class_node.col_vals.len() {
                                    break;
                                }
                            }
                            class_node.children.push(field_node);
                        }
                        root_node.children.push(class_node);
                    }

                    stt.rows.push(root_node);
                    tables.push(stt);
                }

                Ok(PipelineValues::SymbolTreeTableList(SymbolTreeTableList {
                    tables,
                }))
            }
        }
    }
}
