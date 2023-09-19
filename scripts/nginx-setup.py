#!/usr/bin/env python3

# Create our nginx configuration.
#
# The general scheme is `TREE/SERVICE/...` where SERVICE is "source" or
# "raw-analysis" for files available on disk and various dynamic requests that
# get proxied to per-tree local servers running on localhost.
#
# We have a docroot at /home/ubuntu/docroot that provides a place to decide what
# gets exposed in the root of the origin.  It also is used for the "source"
# mapping with symlinks helping map into /home/ubuntu/index/TREE/file (for
# rendered source files) and /home/ubuntu/index/TREE/dir (for rendered directory
# listings), but that could just as easily be accomplished with slightly fancier
# location directives.

from __future__ import absolute_import
from __future__ import print_function
import sys
import json
import os.path
import subprocess
import six

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
  'ttf xpi bcmap icns sqlite jar woff class m4s mgif otf': 'application/x-unknown',
}

binary_types_str = " ".join((mime + " " + exts + ";") for (exts, mime) in six.iteritems(binary_types))

def location(route, directives):
    print(f'  location {route} {{')

    # Use HSTS in release - ELB sets http_x_forwarded_proto, so this
    # won't match in dev builds.  This needs to be included in all
    # locations, instead of in the server block, since add_header
    # won't be inherited if a location sets any headers of its own.
    if use_hsts:
        print('    add_header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload" always;')

    for directive in directives:
        print(f'    {directive}')
        if nginx_cache_dir and 'proxy_pass' in directive:
            print('    proxy_cache sfox;')
            print('    add_header X-Cache-Status $upstream_cache_status;')
    print('  }')
    print()

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
    # - max_size=20g - 20 gigs of cached data, max.  This is a somewhat
    #   arbitrary decision based on the mozilla-releases.json using 223G of 296G
    #   right now, leaving 59G free.
    # - use_temp_path=off - Disables the file being written to disk in one
    #   location and then moved/copied to its final destination.  Recommended.
    # - inactive=7d - Keep the data basically forever until LRU evicted because
    #   the cache has filled up.  The machine should be reaped after 2 days in
    #   normal successful operation, so anything above that is really just a
    #   convenience for analysis purposes.
    print('proxy_cache_path', nginx_cache_dir, 'levels=1:2 keys_zone=sfox:10m max_size=20g use_temp_path=off inactive=7d;')
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
    # XXX cache despite the server saying otherwise
    print('proxy_ignore_headers X-Accel-Expires Expires Cache-Control Set-Cookie;')
    print('')

print('''# we are in the "http" context here.
log_format custom_cache_log '[$time_local] [Cache:$upstream_cache_status] [$request_time] [$host] [Remote_Addr: $remote_addr] - $remote_user - $server_name to: $upstream_addr: "$request" $status $body_bytes_sent "$http_referer" "$http_user_agent" ' ;

map $status $expires {
  default 2m;
  "301" 1m;
}

server {
  listen 80 default_server;

  access_log /var/log/nginx/searchfox.log custom_cache_log ;

  # Redirect HTTP to HTTPS in release
  if ($http_x_forwarded_proto = "http") {
    return 301 https://$host$request_uri;
  }

  sendfile off;

  expires $expires;
  etag on;
''')

# root means "/static" will be appended to the root, versus alias which doesn't.
location('/static', [f'root {mozsearch_path};'])
location('= /robots.txt', [
    f'root {mozsearch_path}/static;',
    'try_files $uri =404;',
    'add_header Cache-Control "public";',
    'expires 1d;',
])

# TODO: it's possible some of the `try_files` machinations and symlinks
# we're using could better cleaned up by use of "alias".  The exception is
# "source" where the "try_files" is definitely absolutely necessary.
for repo in config['trees']:
    tree_config = config['trees'][repo]
    index_path = tree_config['index_path']
    head_rev = None
    if 'git_path' in config['trees'][repo]:
        try:
            head_rev = subprocess.check_output(['git', '--git-dir', config['trees'][repo]['git_path'] + '/.git', 'rev-parse', 'HEAD'], text=True).strip()
        except subprocess.CalledProcessError:
            # If this fails just leave head_rev as None and skip the optimization
            pass

    alias = tree_config['alias']
    if alias:
        location(f'~ /{alias}(.*)$', [
            f'return 301 $scheme://$http_host/{repo}$1;'
        ])

    # we use alias because the we don't want the "/{repo}" portion.
    location(f'/{repo}/static/', [f'alias {mozsearch_path}/static/;'])

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
        location(f'~^/{repo}/rev/{head_rev}/(?<head_path>.+)$', [
            f'root {doc_root}/file/{repo}/source;',
            'try_files /$head_path =404;',
            f'types {{ {binary_types_str} }}',
            'default_type text/html;',
            'add_header Cache-Control "must-revalidate";',
            'gzip_static always;',
            'gunzip on;',
        ])

    # Handled by router/router.py
    location(f'/{repo}/search', ['proxy_pass http://localhost:8000;'])
    location(f'/{repo}/sorch', ['proxy_pass http://localhost:8000;'])
    location(f'/{repo}/define', ['proxy_pass http://localhost:8000;'])

    # Handled by Rust `web-server.rs`.
    location(f'/{repo}/diff', ['proxy_pass http://localhost:8001;'])
    location(f'/{repo}/commit', ['proxy_pass http://localhost:8001;'])
    location(f'/{repo}/rev', ['proxy_pass http://localhost:8001;'])
    location(f'/{repo}/hgrev', ['proxy_pass http://localhost:8001;'])
    location(f'/{repo}/complete', ['proxy_pass http://localhost:8001;'])
    location(f'/{repo}/commit-info', ['proxy_pass http://localhost:8001;'])

    # Handled by Rust `pipeline-server.rs`
    location(f'/{repo}/query', ['proxy_pass http://localhost:8002;'])


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

print('}')
