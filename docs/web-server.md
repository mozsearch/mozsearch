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
unless its config says `"data_api": false`.  See `tools/src/data_api.rs` for
the routes.
