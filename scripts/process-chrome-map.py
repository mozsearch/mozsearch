#!/usr/bin/env python3

import glob
import json
import os
import sys


def process_chrome_map(url_map, chrome_map_path, topsrcdir):
    if not os.path.exists(chrome_map_path):
        return

    with open(chrome_map_path, "r") as f:
        url_prefixes, overrides, install_info, buildconfig = json.load(f)

    # See m-c/python/mozbuild/mozbuild/codecoverage/lcov_rewriter.py.
    if "resource:///" not in url_prefixes:
        url_prefixes["resource:///"] = ["dist/bin/browser"]
    if "resource://gre/" not in url_prefixes:
        url_prefixes["resource://gre/"] = ["dist/bin"]

    reverse_prefixes = {}
    for from_prefix, to_prefixes in url_prefixes.items():
        for to_prefix in to_prefixes:
            reverse_prefixes[to_prefix] = from_prefix


    def map_path(path):
        """Returns all mapped URLs for given path or URL."""
        for from_prefix, to_prefix in reverse_prefixes.items():
            if path.startswith(from_prefix):
                mapped = to_prefix + path[len(from_prefix) + 1:]
                yield mapped

                yield from map_path(mapped)


    def get_overrides(url):
        """Returns all overridden URLs for given URL."""
        for to_name, from_name in overrides.items():
            if from_name == url:
                yield to_name

                yield from get_overrides(to_name)


    def add_entries(url_map, src, obj):
        urls = list(map_path(obj))
        if len(urls) == 0:
            return

        overridden_urls = []
        for url in urls:
            overridden_urls += get_overrides(url)
        urls += overridden_urls

        for url in urls:
            if url not in url_map:
                url_map[url] = []
            if src not in url_map[url]:
                url_map[url].append(src)


    for obj, item in install_info.items():
        src = item[0]

        if "*" in src:
            # The source path is written with glob.
            # Handle all matching files.
            for src_path in glob.glob(src, root_dir=topsrcdir):
                obj_path = os.path.join(obj, os.path.basename(src_path))
                add_entries(url_map, src_path, obj_path)
        else:
            add_entries(url_map, src, obj)


topsrcdir = sys.argv[1]
url_map_path = sys.argv[2]
chrome_map_paths = sys.argv[3:]

url_map = {}
for path in chrome_map_paths:
    process_chrome_map(url_map, path, topsrcdir)

with open(url_map_path, "w") as f:
    json.dump(url_map, f)
