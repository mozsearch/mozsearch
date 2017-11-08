#!/usr/bin/env python

import sys
import json
import os.path

config_fname = sys.argv[1]
doc_root = sys.argv[2]

mozsearch_path = os.path.realpath(os.path.join(os.path.dirname(os.path.realpath(sys.argv[0])), '..'))

config = json.load(open(config_fname))

fmt = {
  'doc_root': doc_root,
  'mozsearch_path': mozsearch_path,
}

def location(route, directives):
    print '  location %s {' % (route % fmt)

    # Use HSTS in release - ELB sets http_x_forwarded_proto, so this
    # won't match in dev builds.  This needs to be included in all
    # locations, instead of in the server block, since add_header
    # won't be inherited if a location sets any headers of its own.
    print '''    if ($http_x_forwarded_proto = "https") {
      add_header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload" always;
    }'''

    for directive in directives:
        print '    ' + (directive % fmt)

    print '  }'
    print

print '''server {
  listen 80 default_server;

  # Redirect HTTP to HTTPS in release
  if ($http_x_forwarded_proto = "http") {
    return 301 https://$host$request_uri;
  }

  sendfile off;
''' % fmt

location('/static', ['root %(mozsearch_path)s;'])

for repo in config['trees']:
    fmt['repo'] = repo

    location('/%(repo)s/source', [
        'root %(doc_root)s;',
        'try_files /file/$uri /dir/$uri/index.html =404;',
        'types { image/png png; image/jpeg jpeg; }',
        'default_type text/html;',
        'expires 1d;',
        'add_header Cache-Control "public";',
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


location('= /', [
    'root %(doc_root)s;',
    'try_files $uri/help.html =404;',
    'expires 1d;',
    'add_header Cache-Control "public";',
])

print '}'
