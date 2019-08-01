#!/usr/bin/env python

import sys
import json
import os.path
import subprocess

config_fname = sys.argv[1]
doc_root = sys.argv[2]
use_hsts = sys.argv[3] == 'hsts'

print '# use_hsts =', sys.argv[3]

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
  'ttf xpi bcmap icns sqlite jar woff class m4s mgif otf': 'application/x-unknown',
}

fmt = {
  'doc_root': doc_root,
  'mozsearch_path': mozsearch_path,
  'binary_types': " ".join((mime + " " + exts + ";") for (exts, mime) in binary_types.iteritems()),
}

def location(route, directives):
    print '  location %s {' % (route % fmt)

    # Use HSTS in release - ELB sets http_x_forwarded_proto, so this
    # won't match in dev builds.  This needs to be included in all
    # locations, instead of in the server block, since add_header
    # won't be inherited if a location sets any headers of its own.
    if use_hsts:
        print '    add_header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload" always;'

    for directive in directives:
        print '    ' + (directive % fmt)

    print '  }'
    print

print '''# we are in the "http" context here.
map $status $expires {
  default 2m;
  "301" 1m;
}

server {
  listen 80 default_server;

  # Redirect HTTP to HTTPS in release
  if ($http_x_forwarded_proto = "http") {
    return 301 https://$host$request_uri;
  }

  sendfile off;

  expires $expires;
  etag on;
''' % fmt

location('/static', ['root %(mozsearch_path)s;'])
location('= /robots.txt', [
    'root %(mozsearch_path)s/static;',
    'try_files $uri =404;',
    'add_header Cache-Control "public";',
    'expires 1d;',
])

for repo in config['trees']:
    head_rev = None
    if 'git_path' in config['trees'][repo]:
        try:
            head_rev = subprocess.check_output(['git', '--git-dir', config['trees'][repo]['git_path'] + '/.git', 'rev-parse', 'HEAD']).strip()
        except subprocess.CalledProcessError:
            # If this fails just leave head_rev as None and skip the optimization
            pass

    fmt['repo'] = repo
    fmt['head'] = head_rev

    location('/%(repo)s/source', [
        'root %(doc_root)s;',
        'try_files /file/$uri /dir/$uri/index.html =404;',
        'types { %(binary_types)s }',
        'default_type text/html;',
        'add_header Cache-Control "must-revalidate";',
    ])

    # Optimization to handle the head revision by serving the file directly instead of going through
    # the rust web-server. This is worth it because when HEAD-rev permalinks are generated they are
    # often hit multiple times while they are still the HEAD revision.
    if head_rev is not None:
        location('~^/%(repo)s/rev/%(head)s/(?<head_path>.+)$', [
            'root %(doc_root)s/file/%(repo)s/source;',
            'try_files /$head_path =404;',
            'types { %(binary_types)s }',
            'default_type text/html;',
            'add_header Cache-Control "must-revalidate";',
        ])

    # Handled by router/router.py
    location('/%(repo)s/search', ['proxy_pass http://localhost:8000;'])
    location('/%(repo)s/define', ['proxy_pass http://localhost:8000;'])

    # Handled by Rust web-server.
    location('/%(repo)s/diff', ['proxy_pass http://localhost:8001;'])
    location('/%(repo)s/commit', ['proxy_pass http://localhost:8001;'])
    location('/%(repo)s/rev', ['proxy_pass http://localhost:8001;'])
    location('/%(repo)s/complete', ['proxy_pass http://localhost:8001;'])
    location('/%(repo)s/commit-info', ['proxy_pass http://localhost:8001;'])

    del fmt['repo']
    del fmt['head']


location('= /', [
    'root %(doc_root)s;',
    'try_files $uri/help.html =404;',
    'add_header Cache-Control "must-revalidate";',
])

location('= /status.txt', [
    'root %(doc_root)s;',
    'try_files $uri =404;',
    'add_header Cache-Control "must-revalidate";',
])

print '}'
