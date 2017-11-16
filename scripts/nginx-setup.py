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

  # Use HSTS in release - ELB sets http_x_forwarded_proto, so this won't match in dev builds.
  # This needs to be included in all locations, instead of in the server block, since
  # add_header won't be inherited if a location sets any headers of its own.
  'add_hsts': '''if ($http_x_forwarded_proto = "https") {
      add_header Strict-Transport-Security "max-age=63072000; includeSubDomains; preload" always;
    }'''
}

print '''server {
  listen 80 default_server;

  # Redirect HTTP to HTTPS in release
  if ($http_x_forwarded_proto = "http") {
    return 301 https://$host$request_uri;
  }

  sendfile off;

  location /static {
    %(add_hsts)s
    root %(mozsearch_path)s;
  }
''' % fmt

for repo in config['trees']:
    fmt['repo'] = repo

    print '''
  location /%(repo)s/source {
    %(add_hsts)s
    root %(doc_root)s;
    try_files /file/$uri /dir/$uri/index.html =404;
    types {
      image/png png;
      image/jpeg jpeg;
    }
    default_type text/html;
    expires 1d;
    add_header Cache-Control "public";
  }

  location /%(repo)s/search {
    %(add_hsts)s
    proxy_pass http://localhost:8000;
  }

  location /%(repo)s/define {
    %(add_hsts)s
    proxy_pass http://localhost:8000;
  }

  location /%(repo)s/diff {
    %(add_hsts)s
    proxy_pass http://localhost:8001;
  }

  location /%(repo)s/commit {
    %(add_hsts)s
    proxy_pass http://localhost:8001;
  }

  location /%(repo)s/rev {
    %(add_hsts)s
    proxy_pass http://localhost:8001;
  }

  location /%(repo)s/complete {
    %(add_hsts)s
    proxy_pass http://localhost:8001;
  }

  location /%(repo)s/commit-info {
    %(add_hsts)s
    proxy_pass http://localhost:8001;
  }''' % fmt

del fmt['repo']
print '''
  location = / {
    %(add_hsts)s
    root %(doc_root)s;
    try_files $uri/help.html =404;
    expires 1d;
    add_header Cache-Control "public";
  }
}
''' % fmt
