#!/usr/bin/env python3

# Create our nginx configuration.
#
# The general scheme is `TREE/SERVICE/...` where SERVICE is "source" or
# "raw-analysis" for files available on disk and various dynamic requests that
# get proxied to per-tree local servers running on localhost.
#
# We have a docroot at /home/ubuntu/docroot that provides a place to decide what
# gets exposed in the root of the origin.  It also is used for the "source"
# mapping with symlinks helping map into /index/TREE/file (for
# rendered source files) and /index/TREE/dir (for rendered directory
# listings), but that could just as easily be accomplished with slightly fancier
# location directives.

import sys
import json
import os.path
import subprocess

# The config file at the root of the WORKING directory; all paths should be
# absolute paths.
config_fname = sys.argv[1]
# doc_root will usually be /home/ubuntu/docroot and will hold files like:
# - status.txt: A file written by the web-servers that the web-server triggering
#   process polls in order to know when the web-server is up and the load
#   balancers can be redirected at.
doc_root = sys.argv[2]
# although these arguments are optional, web-server-setup.sh explicitly passes
# empty values when omitted by wrapping them in quotes.
use_hsts = sys.argv[3] == 'hsts'
nginx_cache_dir = sys.argv[4] # empty string if not specified, which is falsey.
nginx_log_dir = sys.argv[5] # empty string if not specified, which is falsey.

if not nginx_log_dir:
    nginx_log_dir = '/tmp'

print('# use_hsts =', sys.argv[3])

mozsearch_path = os.path.realpath(os.path.join(os.path.dirname(os.path.realpath(sys.argv[0])), '..'))

config = json.load(open(config_fname))

# Keep this list in sync with the FormatAs::Binary list in languages.rs
binary_types = {
  'ogg opus': 'audio/ogg',
  'wav': 'audio/wav',
  'mp3': 'audio/mpeg',
  'png': 'image/png',
  'gif': 'image/gif',
  'jpg jpeg': 'image/jpeg',
  'bmp': 'image/bmp',
  'ico': 'image/vnd.microsoft.icon',
  'ogv': 'video/ogg',
  'mp4': 'video/mpeg',
  'webm': 'video/webm',
  'webp': 'image/webp',
  'ttf xpi bcmap icns sqlite jar woff class m4s mgif otf car': 'application/x-unknown',
}

binary_types_str = " ".join((mime + " " + exts + ";") for (exts, mime) in binary_types.items())

def location(route, directives, cache=True):
    """A location, whose proxied responses are cached unless `cache` is False
    (ex: for what changes, like a try import's status), in which case browsers
    get the server's caching headers too, rather than `expires`'s."""
    print(f'  location {route} {{')
    if not cache:
        print('    expires off;')

    # Use HSTS in release - ELB sets http_x_forwarded_proto, so this
    # won't match in dev builds.  This needs to be included in all
    # locations, instead of in the server block, since add_header
    # won't be inherited if a location sets any headers of its own.
    if use_hsts:
        print('    add_header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload" always;')

    for directive in directives:
        print(f'    {directive}')
        if nginx_cache_dir and cache and 'proxy_pass' in directive:
            print('    proxy_cache sfox;')
            print('    add_header X-Cache-Status $upstream_cache_status;')
    print('  }')
    print()

print('''
pid /tmp/nginx.pid;

events {
    worker_connections 768;
    # multi_accept on;
}

http {
sendfile on;
tcp_nopush on;
types_hash_max_size 2048;

client_body_temp_path /tmp/nginx.body;
proxy_temp_path /tmp/nginx.proxy;
fastcgi_temp_path /tmp/nginx.fastcgi;
uwsgi_temp_path /tmp/nginx.uwsgi;
scgi_temp_path /tmp/nginx.scgi;

types {
    text/html                                        html htm shtml;
    text/css                                         css;
    text/xml                                         xml;
    image/gif                                        gif;
    image/jpeg                                       jpeg jpg;
    application/javascript                           js;
    application/atom+xml                             atom;
    application/rss+xml                              rss;

    text/mathml                                      mml;
    text/plain                                       txt;
    text/vnd.sun.j2me.app-descriptor                 jad;
    text/vnd.wap.wml                                 wml;
    text/x-component                                 htc;

    image/avif                                       avif;
    image/png                                        png;
    image/svg+xml                                    svg svgz;
    image/tiff                                       tif tiff;
    image/vnd.wap.wbmp                               wbmp;
    image/webp                                       webp;
    image/x-icon                                     ico;
    image/x-jng                                      jng;
    image/x-ms-bmp                                   bmp;

    font/woff                                        woff;
    font/woff2                                       woff2;

    application/java-archive                         jar war ear;
    application/json                                 json;
    application/mac-binhex40                         hqx;
    application/msword                               doc;
    application/pdf                                  pdf;
    application/postscript                           ps eps ai;
    application/rtf                                  rtf;
    application/vnd.apple.mpegurl                    m3u8;
    application/vnd.google-earth.kml+xml             kml;
    application/vnd.google-earth.kmz                 kmz;
    application/vnd.ms-excel                         xls;
    application/vnd.ms-fontobject                    eot;
    application/vnd.ms-powerpoint                    ppt;
    application/vnd.oasis.opendocument.graphics      odg;
    application/vnd.oasis.opendocument.presentation  odp;
    application/vnd.oasis.opendocument.spreadsheet   ods;
    application/vnd.oasis.opendocument.text          odt;
    application/vnd.openxmlformats-officedocument.presentationml.presentation
                                                     pptx;
    application/vnd.openxmlformats-officedocument.spreadsheetml.sheet
                                                     xlsx;
    application/vnd.openxmlformats-officedocument.wordprocessingml.document
                                                     docx;
    application/vnd.wap.wmlc                         wmlc;
    application/wasm                                 wasm;
    application/x-7z-compressed                      7z;
    application/x-cocoa                              cco;
    application/x-java-archive-diff                  jardiff;
    application/x-java-jnlp-file                     jnlp;
    application/x-makeself                           run;
    application/x-perl                               pl pm;
    application/x-pilot                              prc pdb;
    application/x-rar-compressed                     rar;
    application/x-redhat-package-manager             rpm;
    application/x-sea                                sea;
    application/x-shockwave-flash                    swf;
    application/x-stuffit                            sit;
    application/x-tcl                                tcl tk;
    application/x-x509-ca-cert                       der pem crt;
    application/x-xpinstall                          xpi;
    application/xhtml+xml                            xhtml;
    application/xspf+xml                             xspf;
    application/zip                                  zip;

    application/octet-stream                         bin exe dll;
    application/octet-stream                         deb;
    application/octet-stream                         dmg;
    application/octet-stream                         iso img;
    application/octet-stream                         msi msp msm;

    audio/midi                                       mid midi kar;
    audio/mpeg                                       mp3;
    audio/ogg                                        ogg;
    audio/x-m4a                                      m4a;
    audio/x-realaudio                                ra;

    video/3gpp                                       3gpp 3gp;
    video/mp2t                                       ts;
    video/mp4                                        mp4;
    video/mpeg                                       mpeg mpg;
    video/ogg                                        ogv;
    video/quicktime                                  mov;
    video/webm                                       webm;
    video/x-flv                                      flv;
    video/x-m4v                                      m4v;
    video/x-matroska                                 mkv;
    video/x-mng                                      mng;
    video/x-ms-asf                                   asx asf;
    video/x-ms-wmv                                   wmv;
    video/x-msvideo                                  avi;
}

default_type application/octet-stream;

ssl_protocols TLSv1 TLSv1.1 TLSv1.2 TLSv1.3;
ssl_prefer_server_ciphers on;

gzip on;
''')

if nginx_cache_dir:
    # Proxy Cache Settings.
    #
    # These are enabled on a per-location basis
    #
    # - levels=1:2 - 2 levels of directories is a ward against file system
    #   slowness with tons of files in a directory.  May not actually be
    #   necessary.
    # - keys_zone=sfox:10m - 10 megs of keys at 8,000 keys per meg is 80,000
    #   keys or 80,000 cache things.  This was a default recommendation that's
    #   expected to be sufficient.  The "sfox" is the name of the cache to be
    #   used with `proxy_cache`.
    # - max_size=18g - 18 gigs of cached data, max.  This is a somewhat
    #   arbitrary decision based on the mozilla-releases.json using 223G of 296G
    #   right now, leaving 59G free.  (It was 20g; the pipeline-server's step
    #   cache, of at most 2 GB, see tools/src/step_cache.rs, gets the rest of
    #   the space index.sh's NGINX_CACHE_IN_KB reserves.)
    #   If you change this value, please update the NGINX_CACHE_IN_KB variable
    #   in infrastructure/aws/index.sh script as well.
    # - use_temp_path=off - Disables the file being written to disk in one
    #   location and then moved/copied to its final destination.  Recommended.
    # - inactive=7d - Keep the data basically forever until LRU evicted because
    #   the cache has filled up.  The machine should be reaped after 2 days in
    #   normal successful operation, so anything above that is really just a
    #   convenience for analysis purposes.
    print('proxy_cache_path', nginx_cache_dir, 'levels=1:2 keys_zone=sfox:10m max_size=18g use_temp_path=off inactive=7d;')
    # If a 2nd identical request comes in while we're still asking the server
    # for an answer, block the 2nd and serve it the result of the 1st.  This is
    # massively desired for cases where anxious users hit the refresh button on
    # a slow-to-load page.
    print('proxy_cache_lock on;')
    # And those worst-cases can be very bad, so choose much longer lock timeouts
    # than 5s.  Note that proxy_read_timeout is still 60s so if the server
    # buffers the whole time, things will still break.
    print('proxy_cache_lock_age 3m;')
    print('proxy_cache_lock_timeout 3m;')
    print('proxy_read_timeout 3m;')
    print('proxy_cache_valid 200 120d;')
    # XXX cache despite the server saying otherwise (but for X-Accel-Expires,
    # which the web-server sends for pages it wrote as sidecar files, which
    # nginx serves instead; see `tools::sidecars`)
    print('proxy_ignore_headers Expires Cache-Control Set-Cookie;')
    print('')

# Peephole histories (see `format::peephole_json` in the tools) keep loading as
# the user scrolls, and they and futures (`format::future_json`) are relatively
# expensive, so limit how fast each client can request them, mainly against
# scrapers.
print('limit_req_zone $binary_remote_addr zone=peephole:10m rate=5r/s;')

print('''# we are in the "http" context here.
log_format custom_cache_log '[$time_local] [Cache:$upstream_cache_status] [$request_time] [$host] [Remote_Addr: $remote_addr] - $remote_user - $server_name to: $upstream_addr: "$request" $status $body_bytes_sent "$http_referer" "$http_user_agent" ' ;

map $host $favicon {
  default            testing.png;
  searchfox.org      production.png;
  localhost          localhost.png;
}

map $status $expires {
  default 2m;
  "301" 1m;
}

# Responses the web-server says nothing may cache (`Cache-Control: no-store`,
# with `X-Accel-Expires: 0` for nginx; ex: a try push's page while it's being
# imported) keep that for browsers too, rather than `$expires`'s.
map $upstream_http_cache_control $no_store_expires {
  "~no-store" off;
  default $expires;
}

# The sidecar files of the web-server's pages (see `tools::sidecars`), which
# requests with queries don't have.
map $args $page_sidecar_html {
  "" "$uri.page.html";
  default "/.no-page-sidecar";
}
map $args $page_sidecar_json {
  "" "$uri.page.json";
  default "/.no-page-sidecar";
}

# router.py's search in a tree searching with queries: pages (ex: its search
# boxes', keyword and OpenSearch engines', and old links) go to the
# web-server, which normalizes what a person searched for into a query (see
# its `search` route), and JSON (ex: scripts') to router.py.
map $http_accept $classic_search_server {
  default 127.0.0.1:8001;
  "~application/json" 127.0.0.1:8000;
}

server {
  listen 16995 default_server;
  absolute_redirect off;
''')

print(f'''
  access_log {nginx_log_dir}/searchfox.log custom_cache_log ;
''')

print('''
  # Redirect HTTP to HTTPS in release
  if ($http_x_forwarded_proto = "http") {
    return 301 https://$host$request_uri;
  }

  sendfile off;

  expires $expires;
  etag on;
''')

# The static files (scripts, styles, icons, and vendored libraries like d3,
# which pages load from here rather than from CDNs) get `expires $expires`'s
# 2 minutes, with their ETags and modification times, and must be revalidated
# after, so that a server update's (or a hot-fix's) files are used within 2
# minutes, by browsers and the CDN in front.
static_cache = ['add_header Cache-Control "must-revalidate";']

# root means "/static" will be appended to the root, versus alias which doesn't.
location('/static', [f'root {mozsearch_path};'] + static_cache)
location('= /robots.txt', [
    f'root {mozsearch_path}/static;',
    'try_files $uri =404;',
    'add_header Cache-Control "public";',
    'expires 1d;',
])

print(' error_page 404 /static/html/404.html;')

# The web-server's sidecar files (see `tools::sidecars`) which aren't on disk
# (yet): it generates them (and writes them), so its responses aren't cached
# too.
location('@sidecar', ['proxy_pass http://127.0.0.1:8001;'], cache=False)
# The web-server's pages which aren't sidecar files (yet), or have queries;
# nginx caches them, but for those it writes as sidecar files (whose
# responses say not to).
location('@page', ['proxy_pass http://127.0.0.1:8001;'])

# The web-server's routes whose pages it writes as sidecar files (keep in sync
# with `tools::sidecars::PAGE_ROUTES`), gzipped with empty markers for
# `try_files`, as `{uri}.page.html` or `{uri}.page.json`, whose extension
# gives their content type.
page_sidecar_routes = [
    'algorithm',
    'commit',
    'commit-info',
    'diagnostics',
    'diff',
    'explore',
    'history',
    'interdiff',
    'oldcommit',
    'olddiff',
    'oldrev',
    'outline-diff',
    'rendered',
    'rev',
    'rev-algorithm',
    'rev-canvas',
    'rev-outline',
    'rev-rendered',
]

# TODO: it's possible some of the `try_files` machinations and symlinks
# we're using could better cleaned up by use of "alias".  The exception is
# "source" where the "try_files" is definitely absolutely necessary.
for repo in config['trees']:
    tree_config = config['trees'][repo]
    index_path = tree_config['index_path']
    # The web-server's sidecar files, which it writes by URL beside the trees'
    # index directories (see `tools::sidecars`).
    sidecar_root = os.path.join(os.path.dirname(index_path), 'sidecars')
    head_rev = None
    if 'git_path' in tree_config:
        try:
            head_rev = subprocess.check_output(['git', '--git-dir', tree_config['git_path'] + '/.git', 'rev-parse', 'HEAD'], text=True).strip()
        except subprocess.CalledProcessError:
            # If this fails just leave head_rev as None and skip the optimization
            pass

    if 'oldtree_name' in tree_config:
        oldtree_name = tree_config['oldtree_name']
        print(f'  rewrite ^/{oldtree_name}/rev/(.*)$ /{repo}/oldrev/$1 permanent;')
        print(f'  rewrite ^/{oldtree_name}/commit/(.*)$ /{repo}/oldcommit/$1 permanent;')
        print(f'  rewrite ^/{oldtree_name}/diff/(.*)$ /{repo}/olddiff/$1 permanent;')
        print(f'  rewrite ^/{oldtree_name}/(.*)$ /{repo}/$1 permanent;')
        print('')

    # we use alias because the we don't want the "/{repo}" portion.
    location(f'/{repo}/static/', [f'alias {mozsearch_path}/static/;'] + static_cache)

    location(f'/{repo}/pages/', [f'alias {index_path}/pages/;'])

    location(f'/{repo}/source', [
        f'root {doc_root};',
        'try_files /file/$uri /dir/$uri/index.html =404;',
        f'types {{ {binary_types_str} }}',
        'default_type text/html;',
        'add_header Cache-Control "must-revalidate";',
        'gzip_static always;',
        'gunzip on;',
    ])

    location(f'/{repo}/raw-analysis', [
        f'root {doc_root};',
        'try_files /raw-analysis/$uri =404;',
        'types { }',
        # I tried serving this as application/x-ndjson but then something weird
        # happened content-encoding-wise.  The received response was content
        # encoded but the response headers didn't express it, so Firefox didn't
        # decode the result.
        'default_type text/plain;',
        'add_header Cache-Control "must-revalidate";',
        'gzip_static always;',
        'gunzip on;',
    ])

    location(f'/{repo}/raw', [
        f'root {doc_root};',
        'try_files /raw/$uri =404;',
        'types { }',
        # We serve all raw files as text/plain because our goal is to provide
        # a readable / downloadable version of any generated files, but we
        # categorically are not "hosting" these files. In particular, if an
        # HTML file showed up in there it must not be served as text/html.
        'default_type text/plain;',
        'add_header Cache-Control "must-revalidate";',
        'gzip_static always;',
        'gunzip on;',
    ])

    # The token-centric blame's data files for the tip's source listings; see
    # `format::hyperblame_files`.  (The web-server generates them for other
    # revisions under `rev-hyperblame`.)
    location(f'/{repo}/hyperblame', [
        f'root {doc_root};',
        'try_files /hyperblame/$uri =404;',
        'types { }',
        'default_type application/json;',
        'add_header Cache-Control "must-revalidate";',
        'gzip_static always;',
        'gunzip on;',
    ])

    location(f'/{repo}/file-lists', [
        f'root {doc_root};',
        'try_files /file-lists/$uri =404;',
        'types { }',
        'default_type text/plain;',
        'add_header Cache-Control "must-revalidate";',
    ])

    # Optimization to handle the head revision by serving the file directly instead of going through
    # the rust web-server. This is worth it because when HEAD-rev permalinks are generated they are
    # often hit multiple times while they are still the HEAD revision.
    if head_rev is not None:
        location(f'~^/{repo}/rev/{head_rev}/(?<head_path>.*)$', [
            f'root {doc_root};',
            f'try_files /file/{repo}/source/$head_path /dir/{repo}/source/$head_path/index.html =404;',
            f'types {{ {binary_types_str} }}',
            'default_type text/html;',
            'add_header Cache-Control "must-revalidate";',
            'gzip_static always;',
            'gunzip on;',
        ])

    # Peephole histories and futures are always generated by the web-server,
    # even for the head revision.  (This must come before the location below.)
    location(f'~^/{repo}/rev-hyperblame/.+/(peephole|future|before)/[0-9,]+\\.json$', [
        'limit_req zone=peephole burst=20 nodelay;',
        'proxy_pass http://127.0.0.1:8001;',
    ])

    # Like the above, the head revision's hyperblame data is the static files.
    if head_rev is not None:
        location(f'~^/{repo}/rev-hyperblame/{head_rev}/(?<head_path>.*)$', [
            f'root {doc_root};',
            f'try_files /hyperblame/{repo}/hyperblame/$head_path =404;',
            'types { }',
            'default_type application/json;',
            'add_header Cache-Control "must-revalidate";',
            'gzip_static always;',
            'gunzip on;',
        ])

    # Handled by router/router.py
    if tree_config.get('search_endpoint') == 'query':
        location(f'/{repo}/search', ['proxy_pass http://$classic_search_server;'])
    else:
        location(f'/{repo}/search', ['proxy_pass http://127.0.0.1:8000;'])
    location(f'/{repo}/sorch', ['proxy_pass http://127.0.0.1:8000;'])
    location(f'/{repo}/define', ['proxy_pass http://127.0.0.1:8000;'])

    # Handled by Rust `web-server.rs`: its pages, which it writes as sidecar
    # files (see above), falling back to it,
    for route in page_sidecar_routes:
        location(f'/{repo}/{route}', [
            f'root {sidecar_root};',
            'try_files $page_sidecar_html $page_sidecar_json @page;',
            'types { text/html html; application/json json; }',
            'gzip_static always;',
            'gunzip on;',
        ])
    # its pages' data, which it writes as sidecar files by URL (gzipped with
    # empty markers for `try_files`), falling back to it,
    location(f'/{repo}/rev-hyperblame', [
        f'root {sidecar_root};',
        'try_files $uri @sidecar;',
        'types { }',
        'default_type application/json;',
        'add_header Cache-Control "must-revalidate";',
        'gzip_static always;',
        'gunzip on;',
    ])
    # and its other routes.
    location(f'/{repo}/hgrev', ['proxy_pass http://127.0.0.1:8001;'])
    # The tree's landing page, rendered at index time (see the tools'
    # `render_landing`), and its name without the slash.
    location(f'= /{repo}/', [
        f'root {index_path}/pages;',
        'try_files /landing.html =404;',
    ])
    location(f'= /{repo}', [f'return 301 /{repo}/;'], cache=False)
    location(f'/{repo}/complete', ['proxy_pass http://127.0.0.1:8001;'])
    # The tree's OpenSearch description, which searches with `/search` (and
    # whose URLs are absolute, so of the request's host; and so not cached),
    # and its suggestions.
    location(f'= /{repo}/opensearch.xml', [
        # (With its port, unlike $host.)
        'proxy_set_header X-Forwarded-Host $http_host;',
        'proxy_pass http://127.0.0.1:8001;',
    ], cache=False)
    location(f'/{repo}/suggest', ['proxy_pass http://127.0.0.1:8001;'])
    # A try push's page, which also takes POSTs asking for its import, and its
    # files' stack strips and excerpts, cached as pages are once the push is
    # imported (which doesn't change), but not before (the web-server says
    # `no-store`, and `X-Accel-Expires: 0`, until then), and its import's
    # status, which changes until it's imported (see the tools' `try_import`).
    location(f'/{repo}/try', [
        'expires $no_store_expires;',
        'proxy_pass http://127.0.0.1:8001;',
    ])
    location(f'/{repo}/try-status', ['proxy_pass http://127.0.0.1:8001;'], cache=False)
    # A tree of specs' pull request summaries, which change when they're
    # synced, and their syncs' POSTs and status (see the tools' `spec_prs`).
    location(f'/{repo}/spec-prs', ['proxy_pass http://127.0.0.1:8001;'], cache=False)

    # Handled by Rust `pipeline-server.rs`
    location(f'/{repo}/query', ['proxy_pass http://127.0.0.1:8002;'])
    # and its data API (see the tools' `data_api`), unless the tree's is off
    # (whose requests then don't reach it).
    if tree_config.get('data_api', True):
        # (Compressed: analysis records are ~12 times smaller gzipped, and
        # remote servers ask for gzip; for the CDN's requests too, by
        # Accept-Encoding.  The pipeline-server serves the tree's analysis
        # files as they're stored, gzipped (see the tools'
        # `normalized_analysis_file`), when asked for gzip, which nginx
        # always does, so that it doesn't parse and serialize them, and
        # nginx decompresses them for clients which don't take gzip.)
        location(f'/{repo}/api/', [
            'proxy_pass http://127.0.0.1:8002;',
            'proxy_set_header Accept-Encoding gzip;',
            'gunzip on;',
            'gzip_types application/json application/x-ndjson text/plain;',
            'gzip_proxied any;',
            'gzip_vary on;',
        ])
        # but for its routes for try pushes' imports and pull request
        # summaries' syncs, which the web-server performs, and whose statuses
        # (and summaries) change.
        location(f'~ ^/{repo}/api/v1/(try-(status|import)|spec-prs(-sync|-sync-status)?)$', ['proxy_pass http://127.0.0.1:8001;'], cache=False)

    # Show different icons for the testing servers.
    print(f'  rewrite ^/{repo}/static/icons/search.png$ /{repo}/static/icons/$favicon permanent;')


location('= /', [
    f'root {doc_root};',
    'try_files $uri/help.html =404;',
    'add_header Cache-Control "must-revalidate";',
])

location('= /index.html', [
    f'root {doc_root};',
    'try_files /help.html =404;',
    'add_header Cache-Control "must-revalidate";',
])

location('= /status.txt', [
    f'root {doc_root};',
    'try_files $uri =404;',
    'add_header Cache-Control "must-revalidate";',
])

location('= /tree-list.js', [
    f'root {doc_root};',
    'try_files $uri =404;',
    'add_header Cache-Control "must-revalidate";',
])

if config.get("allow_webtest"):
    location(f'/tests/webtest', [
        f'root {mozsearch_path};',
        'add_header Cache-Control "no-cache";',
    ])

# close "server"
print('}')

# close "http"
print('}')
