# Web server

Mozsearch serves pages in three ways:

* Nginx, for static resources and the current versions of source files.
* Python server, for search results.
* Rust, for blame information and historical versions of files.

All requests first go to the Nginx server. Based on the URL, it may
router the request to the Python or Rust servers, each of which runs
on its own port. Eventually search results should be moved to the Rust
server for performance.

The `scripts/nginx-setup.py` script generates the configuration file
for Nginx.

## Query limits

Expensive queries (the history greps, `text-history:`, `re-history:` and their
timelines, and `re:`'s fuller engine for patterns livegrep can't run) plan what
they would read from cheap sizes first, and are refused with a 422 and
`{"error": "query-too-expensive", ...}` (or, for a person, the query page saying
why and how the query could cost less) if that's more than the request's tier
allows.  The tiers are by who the request is from (see
`tools/src/query_limits.rs`): `browser`, `agent` and `bot` (by user agent, as
`tools/src/admission.rs` classes them), `authenticated` (for signed-in people,
once the server has sign-in), and `local` (requests without forwarding headers,
ex: tests and searchfox-tool, which aren't limited).  Each tier has defaults
(`QueryLimits::default_for`; ex: a browser's history grep checks up to 2,000
(revision, file) pairs and 256 MiB of their versions a page, and up to 20,000
candidate revisions in all; bots' history greps aren't allowed), which a config
file's top-level `query_limits` overrides per tier and limit, since the config
says what the server is (its instance and its trees):

```json
"query_limits": {
  "browser": { "history_candidates": 4000 },
  "bot": { "history_grep": true, "history_revisions": 1000 }
}
```

and `SEARCHFOX_QUERY_LIMITS` (the same JSON, in the pipeline-server's
environment) overrides those, for operations.  Expensive queries' processing
time is also charged to their client's token bucket (1 token a second), so that
a client's expensive queries slow its next ones.

## Data API

The pipeline-server (`tools/src/bin/pipeline-server.rs`, behind
`/{tree}/query/` and `/{tree}/api/`) also serves each tree's data API:
`/{tree}/api/v1/...` routes returning the index's data as JSON, as the
tools' types serialize it (crossref entries, jumprefs, identifiers, file
lists, sources, analysis records, the lines a revision changes), for tools
working over a remote tree (`RemoteServer`) and other clients; with `rev=`,
of a try push's analysis.  Requests are GET only, with strictly checked
parameters and paths (422 for unknown or malformed ones), and responses are
capped at 4 MiB unless the request's `max_bytes` raises the cap (406
otherwise).  API requests have their own token buckets, charged for their
processing time (see `tools/src/admission.rs`).  It's on for every tree
unless its config says `"data_api": false`.  Its routes for try pushes'
imports (`try-status`, and POSTs to `try-import`) are the web-server's, which
performs the imports, and nginx sends them there uncached.  See
`tools/src/data_api.rs` for the routes.
