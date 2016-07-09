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
