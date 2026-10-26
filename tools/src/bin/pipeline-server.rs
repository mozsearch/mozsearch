use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    env,
    sync::Arc,
};

use axum::{
    Extension, Json, Router,
    extract::{Path, Query, Request},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::get,
};
use axum_macros::debug_handler;
use liquid::Template;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tools::{
    abstract_server::{
        AbstractServer, ServerError, make_all_local_servers, make_remote_servers,
        remote_trees_config, sym_info,
    },
    admission,
    cmd_pipeline::{
        PipelineValues,
        builder::build_pipeline_graph,
        facets::{CtxtraFacet, FacetFile, PathKinds, PpFacet, file_facets, last_changed_facet},
        interface::FlattenedResultsBundle,
    },
    data_api,
    file_format::config::{self, SearchEndpoint},
    file_format::jumpref::{JumprefData, JumprefTraversals},
    file_format::recency::Recency,
    logging::{LoggedSpan, init_logging},
    memory_budget,
    query::{chew_query::chew_query, dwim::dwim_target, normalize::normalize_input},
    query_limits::{self, Tier},
    templating::builder::build_and_parse_query_results,
};
use tower::limit::GlobalConcurrencyLimitLayer;
use tracing::Instrument;
use ustr::{Ustr, UstrMap};

/// The facets of the files of file-centric results (see facet_bar.liquid), as
/// on the `/explore/` pages: their path kinds, subsystems, and directories, as
/// `{"facets": [...], "files": {PATH: {facets, groups, title}}, "lenses":
/// [...]}`.  And the facets of their lines, "Last changed" and
/// "Preprocessor", if they have them, whose values the lines have (see
/// query_results/line_span.liquid), and their files the union of theirs; see
/// `FlattenedLineSpan::facets`.  And the lenses showing more of the results
/// (see navigation_panel.liquid), if any (or null): "changes", the lines'
/// history digests, if they have them.
fn results_file_facets(
    server: &(dyn AbstractServer + Send + Sync),
    results: &mut FlattenedResultsBundle,
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
    let mut add_file_values = |key: &str, file_values: HashMap<String, Vec<String>>| {
        for (path, values) in file_values {
            if let Some(file_data) = data.get_mut(&path) {
                let mut file_facets: serde_json::Map<String, Value> =
                    serde_json::from_str(&file_data.facets).unwrap_or_default();
                file_facets.insert(key.to_string(), json!(values));
                file_data.facets = Value::Object(file_facets).to_string();
            }
        }
    };

    // The "Last changed" facet of the results' lines (and file name matches,
    // which don't have history digests).
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
            add_file_values("recency", file_values);
            true
        }
        None => false,
    };

    // The "Preprocessor" facet of the results' lines (not file name matches,
    // which aren't lines).
    let lines = results.path_kind_results.iter().flat_map(|pk_group| {
        pk_group.kind_groups.iter().flat_map(|kind_group| {
            kind_group.by_file.iter().flat_map(|file| {
                file.line_spans
                    .iter()
                    .map(move |span| (file.file.as_str(), span.pp.as_deref()))
            })
        })
    });
    let pp = PpFacet::new(lines).map(|(facet, file_values)| {
        add_file_values("pp", file_values);
        facet
    });

    // The "Placement" facet of the results' lines (see `CtxtraFacet`).
    let lines = results.path_kind_results.iter().flat_map(|pk_group| {
        pk_group.kind_groups.iter().flat_map(|kind_group| {
            kind_group.by_file.iter().flat_map(|file| {
                file.line_spans
                    .iter()
                    .map(move |span| (file.file.as_str(), span.ctxtra.as_slice()))
            })
        })
    });
    let ctxtra = CtxtraFacet::new(lines).map(|(facet, file_values)| {
        add_file_values("ctxtra", file_values);
        facet
    });

    if recency || pp.is_some() || ctxtra.is_some() {
        for pk_group in &mut results.path_kind_results {
            for kind_group in &mut pk_group.kind_groups {
                for file in &mut kind_group.by_file {
                    for span in &mut file.line_spans {
                        let mut line_facets = serde_json::Map::new();
                        if recency {
                            line_facets.insert(
                                "recency".to_string(),
                                json!([Recency::last_changed(span.recency.as_ref())]),
                            );
                        }
                        if let Some(pp) = &pp {
                            line_facets.insert(
                                "pp".to_string(),
                                json!(pp.line_values(&file.file, span.pp.as_deref())),
                            );
                        }
                        if let Some(ctxtra) = &ctxtra {
                            line_facets.insert(
                                "ctxtra".to_string(),
                                json!(ctxtra.line_values(&span.ctxtra)),
                            );
                        }
                        span.facets = Some(Value::Object(line_facets).to_string());
                    }
                }
            }
        }
    }
    if let Some(pp) = pp {
        facets.push(pp.view);
    }
    if let Some(ctxtra) = ctxtra {
        facets.push(ctxtra.view);
    }
    let lenses = if recency {
        json!([{
            "key": "changes",
            "name": "changes",
            "title": "Show how much each result's history changed when, newest first (its history digest)",
        }])
    } else {
        Value::Null
    };
    json!({ "facets": facets, "files": data, "lenses": lenses })
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
    let mut syms: BTreeSet<Ustr> = BTreeSet::new();
    for pk_group in &results.path_kind_results {
        for kind_group in &pk_group.kind_groups {
            for file in &kind_group.by_file {
                for span in &file.line_spans {
                    syms.extend(sym_info::html_symbols(&span.contents));
                }
            }
        }
    }

    // (Like `format::format_code`'s.)
    let mut sym_info: BTreeMap<Ustr, Option<JumprefData>> = BTreeMap::new();
    let mut traversed: UstrMap<JumprefTraversals> = UstrMap::default();
    sym_info::add_syms_info(server, syms, &mut sym_info, &mut traversed).await;
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
    class: Option<Extension<admission::RequestClass>>,
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

    // A query in a try push's context, `rev` being the head of its stack
    // rebased onto the indexed revision, is of the tree as the push changes
    // it (see `try_delta`).
    let rev = params.get("rev").filter(|rev| !rev.is_empty());
    let server: Box<dyn AbstractServer + Send + Sync> = match rev {
        Some(rev) => match server.try_overlay(rev) {
            Some(overlay) => overlay,
            None => {
                return Ok((
                    StatusCode::NOT_FOUND,
                    format!("No semantic analysis of a try push for {}", rev),
                )
                    .into_response());
            }
        },
        None => server.clonify(),
    };
    let try_meta = server.try_delta_meta();

    let accept = headers
        .get("accept")
        .map(|x| x.to_str().unwrap_or("text/html"));
    let make_html = !matches!(accept, Some("application/json"));

    let mut graph = {
        let _log_entered = logged_span
            .as_ref()
            .map(|lspan| lspan.span.clone().entered());

        let pipeline_plan = chew_query(query)?;

        build_pipeline_graph(server.clonify(), pipeline_plan)?
    };
    // (What its commands may cost the server, by who it's from; see
    // `query_limits`.)
    let tier = Tier::of(class.as_deref().copied());
    let expensive = graph.set_limits(&query_limits::limits_for(tier), tier);

    // (Its share of the memory budget, while it runs, by its estimate; a
    // bot's from outside is turned away if the budget can't have it now,
    // rather than wait; see `memory_budget`.)
    let budget = memory_budget::budget();
    let estimate = graph.estimate();
    let share = match class.as_deref() {
        Some(admission::RequestClass::Bot) => match budget.try_acquire(estimate) {
            Some(share) => share,
            None => {
                println!("Turned away a bot's {:?} for the memory budget", query);
                return Ok((
                    StatusCode::TOO_MANY_REQUESTS,
                    [(axum::http::header::RETRY_AFTER, "30")],
                    "The server is busy with other expensive queries.",
                )
                    .into_response());
            }
        },
        _ => budget.acquire(estimate).await,
    };
    let memory_before = memory_budget::cgroup_memory_mib();

    let run_start = std::time::Instant::now();
    let run = match &logged_span {
        Some(lspan) => graph.run(true).instrument(lspan.span.clone()).await,
        _ => graph.run(true).await,
    };
    // (An expensive query's time is charged to its client; see
    // `admission::Charge`.)
    let charge = expensive.then(|| {
        admission::Charge(
            run_start.elapsed().as_secs_f64() * admission::EXPENSIVE_TOKENS_PER_SECOND,
        )
    });
    // (A query too expensive for its tier is a 422, with the page saying
    // why, for a person; see `query_limits`.)
    let mut too_expensive = None;
    let mut result = match run {
        Ok(result) => result,
        Err(ServerError::TooExpensive(refused)) => {
            println!("Refused {:?} ({:?}): {}", query, tier, refused.message());
            if !make_html {
                return Ok(ServerError::TooExpensive(refused).into_response());
            }
            too_expensive = Some(refused.to_json());
            PipelineValues::Void
        }
        Err(err) => return Err(err),
    };
    if share.mib > 0 {
        println!(
            "Memory budget: {} of {} MiB for {:?} (waited {:.1} s; cgroup {:?} -> {:?} MiB)",
            share.mib,
            budget.mib(),
            query,
            share.waited.as_secs_f64(),
            memory_before,
            memory_budget::cgroup_memory_mib()
        );
    }

    // Where a DWIM rule says what a person searched for may be (see
    // `tools::query::dwim`; ex: a GitHub URL of a file, the tree's file), for
    // the page of their search (with its input; see web-server.rs's `search`
    // route), which goes there if the search finds nothing.  (The rules that
    // go at once, its `search` route has gone by; and lookups, which need the
    // trees' configs, are its.)
    let dwim = params.get("input").and_then(|input| {
        dwim_target(
            &tree,
            input,
            &|path: &str| server.file_facet_info(path).is_some(),
            &|_, _| None,
        )
    });
    if make_html
        && too_expensive.is_none()
        && let Some(target) = &dwim
        && finds_nothing(&result)
    {
        return Ok((StatusCode::FOUND, [(header::LOCATION, target.url.clone())]).into_response());
    }

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

        // (The facets first, since inlining the contexts clears the lines'
        // conditionals.)
        let file_facets = match &mut result {
            PipelineValues::FlattenedResultsBundle(results) => {
                results_file_facets(server.as_ref(), results)
            }
            _ => Value::Null,
        };
        if let PipelineValues::FlattenedResultsBundle(results) = &mut result {
            results.inline_contexts(&tree, rev.map(String::as_str));
            results.link_line_numbers(&tree, rev.map(String::as_str));
            results.add_recency_cells();
        }

        // For simplicity, the template expects "results" variable to always be
        // an array.
        // Use an empty array for the void result, which is used when the
        // query is an empty string.
        let result_value = match result {
            PipelineValues::Void => json!([]),
            _ => serde_json::to_value(result).unwrap(),
        };

        // What the query was before it was normalized (by `/search`; see
        // web-server.rs's `query_url`), and what normalizing changed, if it's
        // reproducible (else the input is only shown).
        let interpretation = match params.get("input").filter(|input| input.trim() != query) {
            Some(input) => {
                let normalization = normalize_input(input);
                let changes = match &normalization.query == query {
                    true => normalization.changes,
                    false => vec![],
                };
                json!({ "input": input, "changes": changes })
            }
            None => Value::Null,
        };

        let globals = liquid::object!({
            "results": result_value,
            "query": query.clone(),
            "interpretation": interpretation,
            "dwim": dwim.map(|target| json!({ "url": target.url, "description": target.description })),
            "preset": preset.clone(),
            "tree": tree.clone(),
            // (The page's tree's, for its own results; see line_span.liquid.)
            "search_tree": tree.clone(),
            "search_url": config::search_url(&tree),
            "query_search": config::search_endpoint(&tree) == SearchEndpoint::Query,
            // The try push whose context the query is in, if any.
            "search_rev": rev.cloned().unwrap_or_default(),
            "try_push": try_meta.as_ref().map(|meta| meta.push.clone()).unwrap_or_default(),
            "try_base": try_meta.as_ref().map(|meta| meta.base.clone()).unwrap_or_default(),
            "logs": logs,
            "debug": maybe_log,
            "SYM_INFO_STR": sym_info_str,
            "file_facets": file_facets,
            "too_expensive": too_expensive.clone().unwrap_or(Value::Null),
        });

        let output = templates.query_results.render(&globals)?;
        let status = match too_expensive {
            Some(_) => StatusCode::UNPROCESSABLE_ENTITY,
            None => StatusCode::OK,
        };
        let mut response = (status, header_map, Html(output)).into_response();
        if let Some(charge) = charge {
            response.extensions_mut().insert(charge);
        }
        Ok(response)
    } else {
        let mut response = (header_map, Json(result)).into_response();
        if let Some(charge) = charge {
            response.extensions_mut().insert(charge);
        }
        Ok(response)
    }
}

/// Whether a query's results are none.
fn finds_nothing(result: &PipelineValues) -> bool {
    match result {
        PipelineValues::Void => true,
        PipelineValues::FlattenedResultsBundle(bundle) => bundle
            .path_kind_results
            .iter()
            .all(|results| results.file_names.is_empty() && results.kind_groups.is_empty()),
        _ => false,
    }
}

struct SomeTemplates {
    query_results: Template,
}

/// Requests from outside wait for their client's tokens, or are turned away,
/// and agents and bots only get some of the concurrency limit's permits; and
/// then they're charged for their processing time, for the pools charging
/// for it (the data API's); see `tools::admission`.
async fn admit(mut req: Request, next: Next) -> Response {
    match admission::admit_request(req.headers(), req.uri().path()).await {
        Ok(admitted) => {
            // (Its class, if it's from outside, for its share of the memory
            // budget; see `handle_query`.)
            if admitted.from_outside() {
                req.extensions_mut().insert(admitted.class);
            }
            let response = next.run(req).await;
            if let Some(admission::Processing(processing)) = response.extensions().get() {
                admitted.charge(*processing);
            }
            if let Some(admission::Charge(tokens)) = response.extensions().get() {
                admitted.charge_tokens(*tokens);
            }
            response
        }
        Err(retry) => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, retry.as_secs().to_string())],
            "Too many requests; please slow down.",
        )
            .into_response(),
    }
}

/// Time the request's processing (inside the concurrency limit), for `admit`.
async fn time_processing(req: Request, next: Next) -> Response {
    let start = std::time::Instant::now();
    let mut response = next.run(req).await;
    response
        .extensions_mut()
        .insert(admission::Processing(start.elapsed()));
    response
}

#[tokio::main]
async fn main() {
    init_logging();

    // The config file of the trees to serve, or for a local server over
    // remote trees (ex: for developing their pages' features without indexing
    // them), their URLs (see `make_remote_servers`), with their responses
    // cached where SEARCHFOX_REMOTE_CACHE says, if anywhere.
    let trees = env::args().nth(1).unwrap();
    let (local_servers, data_api_trees) = if trees.contains("://") {
        let cache = env::var("SEARCHFOX_REMOTE_CACHE").ok();
        let servers = make_remote_servers(&trees, cache.as_deref()).unwrap();
        // (For where their pages search.)
        remote_trees_config(&servers, "");
        // (Their data APIs are their servers'.)
        (Arc::new(servers), BTreeSet::new())
    } else {
        let local_servers = Arc::new(make_all_local_servers(&trees).unwrap());
        // Cache steps' results beside the trees' index directories (see
        // `step_cache`).
        let config = config::load(&trees, false, None, None, None);
        // (What queries may cost, by who they're from; see `query_limits`.)
        query_limits::init(config.query_limits.clone());
        if let Some(tree) = config.trees.values().next() {
            let root =
                tools::sidecars::sidecar_root(&tree.paths.index_path).with_file_name("step-cache");
            tools::step_cache::init(&root, tools::step_cache::CAP_BYTES);
        }
        // (The trees whose data API is on; see `TreeConfigPaths::data_api`.)
        let data_api_trees = config
            .trees
            .iter()
            .filter(|(_, tree)| tree.paths.data_api)
            .map(|(name, _)| name.clone())
            .collect();
        (local_servers, data_api_trees)
    };
    // (The budget for its requests' expected memory; see `memory_budget`.)
    println!("Memory budget: {} MiB", memory_budget::budget().mib());
    let templates = Arc::new(SomeTemplates {
        query_results: build_and_parse_query_results(),
    });

    let app = Router::new()
        .route("/{tree}/query/{preset}", get(handle_query))
        .layer(Extension(local_servers.clone()))
        .layer(Extension(templates))
        .merge(data_api::router(local_servers, data_api_trees))
        .layer(middleware::from_fn(time_processing))
        .layer(GlobalConcurrencyLimitLayer::new(4))
        // (Outside the concurrency limit, so that requests waiting for their
        // tokens don't hold its permits.)
        .layer(middleware::from_fn(admit));

    // (Elsewhere with PIPELINE_SERVER_ADDR, ex: beside a production server's
    // on a development machine.)
    let addr = env::var("PIPELINE_SERVER_ADDR").unwrap_or_else(|_| "0.0.0.0:8002".to_string());
    let listener = TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
