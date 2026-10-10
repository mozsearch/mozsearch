#!/usr/bin/env python3

import glob
import json
import re
import os
import sys


# Localization directories, whose files are Fluent resources with ids that are
# their paths under the locale, ex: browser/preferences/preferences.ftl is
# dist/bin/browser/localization/en-US/browser/preferences/preferences.ftl.
LOCALIZATION_RE = re.compile(r"(?:^|/)localization/[^/]+/(.+)$")


def add_localization_entries(resource_map, resource_id, src, topsrcdir):
    """Map the Fluent resources which `src` (a file, or a glob like
    browser/locales/en-US/browser/**/*.ftl) installs as `resource_id` (or
    under it, for a glob: matches go at their paths under the glob's literal
    prefix)."""
    if "*" not in src:
        matches = [(src, resource_id)]
    else:
        base = src[: src.index("*")].rpartition("/")[0]
        matches = [
            (path, resource_id + "/" + os.path.relpath(path, base))
            for path in glob.glob(src, root_dir=topsrcdir, recursive=True)
        ]
    for path, id in matches:
        if path.endswith(".ftl"):
            resource_map.setdefault(id, set()).add(path)


def process_chrome_map(url_map, resource_map, chrome_map_path, topsrcdir):
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

        if m := LOCALIZATION_RE.search(obj):
            add_localization_entries(resource_map, m.group(1), src, topsrcdir)

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
resource_map = {}
for path in chrome_map_paths:
    process_chrome_map(url_map, resource_map, path, topsrcdir)


def atEscape(text):
  return re.sub("[^A-Za-z0-9_/]", lambda m: "@" + "{:02X}".format(ord(m.group(0))), text)


alias_map = {}

for url, paths in url_map.items():
    aliases = []
    for path in paths:
        aliases.append({
            "pretty": "file " + path,
            "sym": "FILE_" + atEscape(path),
        })

    sym = "URL_" + atEscape(url)

    alias_map[sym] = aliases

# Fluent resources, for replace-aliases.py like URLs.
for resource_id, paths in resource_map.items():
    alias_map["FTLRES_" + atEscape(resource_id)] = [
        {
            "pretty": "file " + path,
            "sym": "FILE_" + atEscape(path),
        }
        for path in sorted(paths)
    ]

with open(url_map_path, "w") as f:
    json.dump(alias_map, f)
