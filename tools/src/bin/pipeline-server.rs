use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    env,
    sync::Arc,
};

use axum::{
    Extension, Json, Router,
    extract::{Path, Query},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_macros::debug_handler;
use liquid::Template;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tools::{
    abstract_server::{AbstractServer, ServerError, make_all_local_servers},
    cmd_pipeline::{
        PipelineValues,
        builder::build_pipeline_graph,
        facets::{FacetFile, PathKinds, file_facets, last_changed_facet},
        interface::FlattenedResultsBundle,
    },
    file_format::jumpref::{
        JumprefData, JumprefTraversals, determine_desired_extra_syms_from_jumpref,
        extra_syms_next_step_lookups,
    },
    file_format::recency::Recency,
    logging::{LoggedSpan, init_logging},
    query::chew_query::chew_query,
    templating::builder::build_and_parse_query_results,
};
use tower::limit::GlobalConcurrencyLimitLayer;
use tracing::Instrument;
use ustr::{Ustr, UstrMap, ustr};

/// The facets of the files of file-centric results (see facet_bar.liquid), as
/// on the `/explore/` pages: their path kinds, subsystems, and directories, as
/// `{"facets": [...], "files": {PATH: {facets, groups, title}}}`.
fn results_file_facets(
    server: &(dyn AbstractServer + Send + Sync),
    results: &FlattenedResultsBundle,
) -> Value {
    let mut seen = HashSet::new();
    let mut files = vec![];
    for pk_group in &results.path_kind_results {
        let paths = pk_group.file_names.iter().chain(
            pk_group
                .kind_groups
                .iter()
                .flat_map(|kind_group| kind_group.by_file.iter().map(|file| &file.file)),
        );
        for path in paths {
            if !seen.insert(*path) {
                continue;
            }
            let info = server.file_facet_info(path);
            files.push(FacetFile {
                path: path.to_string(),
                kind: pk_group.path_kind,
                known: info.is_some(),
                subsystem: info.and_then(|(_, subsystem)| subsystem),
            });
        }
    }
    let (mut facets, mut data) = file_facets(&files, &PathKinds(server.path_kinds()));

    // The "Last changed" facet of the results' lines (and file name matches,
    // which don't have history digests), whose values the lines have (see
    // query_results/line_span.liquid), and their files the union of theirs.
    let lines = results.path_kind_results.iter().flat_map(|pk_group| {
        pk_group
            .file_names
            .iter()
            .map(|path| (path.as_str(), results.file_recency.get(path)))
            .chain(pk_group.kind_groups.iter().flat_map(|kind_group| {
                kind_group.by_file.iter().flat_map(|file| {
                    file.line_spans
                        .iter()
                        .map(move |span| (file.file.as_str(), span.recency.as_ref()))
                })
            }))
    });
    let recency = match last_changed_facet(lines) {
        Some((facet, file_values)) => {
            facets.push(facet);
            for (path, values) in file_values {
                if let Some(file_data) = data.get_mut(&path) {
                    let mut file_facets: serde_json::Map<String, Value> =
                        serde_json::from_str(&file_data.facets).unwrap_or_default();
                    file_facets.insert("recency".to_string(), json!(values));
                    file_data.facets = Value::Object(file_facets).to_string();
                }
            }
            true
        }
        None => false,
    };
    json!({ "facets": facets, "files": data, "recency": recency })
}

/// The SYM_INFO (see `format::format_code`) of file-centric results: the
/// jumprefs of the symbols in their excerpts' `data-symbols`, and of the extra
/// symbols the context menu uses (ex: an XPIDL method's C++ binding), so that
/// clicking a symbol in them works like in a source listing.  But without the
/// structured information's platform variants and methods, which nothing on
/// the page uses, and which are most of it for big classes (ex: 17.5 of the
/// 22 MB for "nsIPrincipal" on firefox, with 2 MB for `Document`).
const UNUSED_META: [&str; 2] = ["variants", "methods"];

async fn results_sym_info(
    server: &(dyn AbstractServer + Send + Sync),
    results: &FlattenedResultsBundle,
) -> String {
    const DATA_SYMBOLS: &str = "data-symbols=\"";
    let mut syms: BTreeSet<Ustr> = BTreeSet::new();
    for pk_group in &results.path_kind_results {
        for kind_group in &pk_group.kind_groups {
            for file in &kind_group.by_file {
                for span in &file.line_spans {
                    let mut rest = span.contents.as_str();
                    while let Some(start) = rest.find(DATA_SYMBOLS) {
                        rest = &rest[start + DATA_SYMBOLS.len()..];
                        let end = rest.find('"').unwrap_or(rest.len());
                        syms.extend(rest[..end].split(',').filter(|s| !s.is_empty()).map(ustr));
                        rest = &rest[end..];
                    }
                }
            }
        }
    }

    // (Like `format::format_code`'s.)
    let mut sym_info: BTreeMap<Ustr, Option<JumprefData>> = BTreeMap::new();
    let mut traversed: UstrMap<JumprefTraversals> = UstrMap::default();
    for sym in syms {
        if sym_info.contains_key(&sym) {
            continue;
        }
        let Ok(jumpref) = server.jumpref_lookup(&sym).await else {
            continue;
        };
        let mut extra_syms = determine_desired_extra_syms_from_jumpref(jumpref.as_ref());
        traversed
            .entry(sym)
            .and_modify(|t| *t |= JumprefTraversals::NormalExtra)
            .or_insert(JumprefTraversals::NormalExtra);
        while let Some((extra_sym, next_step)) = extra_syms.pop() {
            if let Some(extra_traversed) = traversed.get_mut(&extra_sym) {
                if extra_traversed.contains(next_step) {
                    continue;
                }
                *extra_traversed |= next_step;
                if let Some(extra_jumpref) = sym_info.get(&extra_sym) {
                    extra_syms.extend(extra_syms_next_step_lookups(
                        extra_jumpref.as_ref(),
                        next_step,
                    ));
                }
            } else if let Ok(extra_jumpref) = server.jumpref_lookup(&extra_sym).await {
                if !next_step.is_empty() {
                    extra_syms.extend(extra_syms_next_step_lookups(
                        extra_jumpref.as_ref(),
                        next_step,
                    ));
                }
                traversed.insert(extra_sym, next_step);
                sym_info.insert(extra_sym, extra_jumpref);
            }
        }
        sym_info.insert(sym, jumpref);
    }
    let mut sym_info = serde_json::to_value(&sym_info).unwrap_or_default();
    if let Some(sym_info) = sym_info.as_object_mut() {
        for jumpref in sym_info.values_mut() {
            // (The history digests are in the results already.)
            if let Some(jumpref) = jumpref.as_object_mut() {
                jumpref.remove("recency");
                jumpref.remove("recency_from");
            }
            if let Some(meta) = jumpref.get_mut("meta").and_then(Value::as_object_mut) {
                for key in UNUSED_META {
                    meta.remove(key);
                }
            }
        }
    }
    sym_info.to_string()
}

#[debug_handler]
async fn handle_query(
    local_servers: Extension<Arc<BTreeMap<String, Box<dyn AbstractServer + Send + Sync>>>>,
    templates: Extension<Arc<SomeTemplates>>,
    headers: HeaderMap,
    Path((tree, preset)): Path<(String, String)>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Response, ServerError> {
    let server = match local_servers.get(&tree) {
        Some(s) => s,
        None => {
            return Ok((StatusCode::NOT_FOUND, format!("No such tree: {}", tree)).into_response());
        }
    };

    if preset.as_str() != "default" {
        return Ok((StatusCode::NOT_FOUND, format!("No such preset: {}", preset)).into_response());
    }

    let maybe_log = params.contains_key("debug");
    let logged_span: Option<LoggedSpan> = if maybe_log {
        Some(LoggedSpan::new_logged_span("query"))
    } else {
        None
    };

    let query = match params.get("q") {
        Some(q) => q,
        None => {
            return Ok((StatusCode::BAD_REQUEST, "No 'q' parameter, no results!").into_response());
        }
    };

    let graph = {
        let _log_entered = logged_span
            .as_ref()
            .map(|lspan| lspan.span.clone().entered());

        let pipeline_plan = chew_query(query)?;

        build_pipeline_graph(server.clonify(), pipeline_plan)?
    };

    let mut result = match &logged_span {
        Some(lspan) => graph.run(true).instrument(lspan.span.clone()).await?,
        _ => graph.run(true).await?,
    };

    let accept = headers
        .get("accept")
        .map(|x| x.to_str().unwrap_or("text/html"));
    let make_html = !matches!(accept, Some("application/json"));

    let logs = match logged_span {
        Some(lspan) => lspan.retrieve_serde_json().await,
        _ => Value::Null,
    };

    // There are a bunch of ways to return headers to axum; this is the most
    // legible I found.
    let mut header_map = HeaderMap::new();
    header_map.insert(header::VARY, "Accept".parse().unwrap());

    if make_html {
        let sym_info_str = match &result {
            PipelineValues::GraphResultsBundle(grb) => {
                serde_json::to_string(&grb.symbols).unwrap_or_else(|_| "{}".to_string())
            }
            PipelineValues::GraphInput(graphs) => {
                serde_json::to_string(&graphs.symbols).unwrap_or_else(|_| "{}".to_string())
            }
            PipelineValues::SymbolTreeTableList(sttl) => {
                serde_json::to_string(&sttl.unioned_node_sets_as_jumprefs())
                    .unwrap_or_else(|_| "{}".to_string())
            }
            PipelineValues::FlattenedResultsBundle(results) => {
                results_sym_info(server.as_ref(), results).await
            }
            _ => "{}".to_string(),
        };

        if let PipelineValues::FlattenedResultsBundle(results) = &mut result {
            results.inline_contexts(&tree);
            results.link_line_numbers(&tree);
            results.add_recency_cells();
        }
        let file_facets = match &result {
            PipelineValues::FlattenedResultsBundle(results) => {
                results_file_facets(server.as_ref(), results)
            }
            _ => Value::Null,
        };
        if let (PipelineValues::FlattenedResultsBundle(results), Some(true)) =
            (&mut result, file_facets["recency"].as_bool())
        {
            for span in results.line_spans_mut() {
                span.last_changed = Some(Recency::last_changed(span.recency.as_ref()));
            }
        }

        // For simplicity, the template expects "results" variable to always be
        // an array.
        // Use an empty array for the void result, which is used when the
        // query is an empty string.
        let result_value = match result {
            PipelineValues::Void => json!([]),
            _ => serde_json::to_value(result).unwrap(),
        };

        let globals = liquid::object!({
            "results": result_value,
            "query": query.clone(),
            "preset": preset.clone(),
            "tree": tree.clone(),
            "logs": logs,
            "debug": maybe_log,
            "SYM_INFO_STR": sym_info_str,
            "file_facets": file_facets,
        });

        let output = templates.query_results.render(&globals)?;
        Ok((header_map, Html(output)).into_response())
    } else {
        Ok((header_map, Json(result)).into_response())
    }
}

struct SomeTemplates {
    query_results: Template,
}

#[tokio::main]
async fn main() {
    init_logging();

    let local_servers = Arc::new(make_all_local_servers(&env::args().nth(1).unwrap()).unwrap());
    let templates = Arc::new(SomeTemplates {
        query_results: build_and_parse_query_results(),
    });

    // build our application with a single route
    let app = Router::new()
        .route("/{tree}/query/{preset}", get(handle_query))
        .layer(Extension(local_servers))
        .layer(Extension(templates))
        .layer(GlobalConcurrencyLimitLayer::new(4));

    let listener = TcpListener::bind("0.0.0.0:8002").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
