#!/usr/bin/env python3

import json
import glob
import os
import re
import sys

analysis_dir = sys.argv[1]
alias_map_path = sys.argv[2]


def at_escape(text):
  return re.sub("[^A-Za-z0-9_/]", lambda m: "@" + "{:02X}".format(ord(m.group(0))), text)


def at_unescape(text):
  return re.sub("@[0-9A-F][0-9A-F]", lambda m: chr(int(m.group(0)[1:], 16)), text)


def resolve_relpath(url, relpath):
    if url.startswith("chrome://"):
        prefix = "chrome:/"
        path = url[8:]
    elif url.startswith("resource://"):
        prefix = "resource:/"
        path = url[10:]

    parent = os.path.dirname(path)
    resolved = os.path.normpath(os.path.join(parent, relpath))

    return prefix + resolved


def replace_aliases(path, alias_map, reverse_map):
    """Replace URL records and RELPATH records with FILE records, based on
    the mapping extracted from *.chrome-map.json files.

    Note that the analysis record transformations done below are only safe for
    URL records which are currently never referenced for contextsym purposes
    or by structured records. Any more involved transformations should likely
    happen in rust code where we have the analysis.rs types available and can
    easily add helper transforms."""

    has_alias = False
    lines = []

    fullpath = os.path.join(analysis_dir, path)

    with open(fullpath, "r") as f:
        for line in f:
            line = line.rstrip()

            # Filter out definitely non-target lines.
            if "URL_" not in line and "RELPATH" not in line:
                lines.append(line)
                continue

            datum = json.loads(line)
            sym = datum["sym"]

            # Actually test if the symbol is the target.
            if not sym.startswith("URL_") and sym != "RELPATH":
                lines.append(line)
                continue

            handled_syms = set()

            has_alias = True


            def handle_url_sym(sym):
                if sym not in alias_map:
                    return

                for alias in alias_map[sym]:
                    datum["sym"] = alias["sym"]
                    datum["pretty"] = alias["pretty"]

                    # NOTE: A file can have multiple URLs, and resolving a
                    #       relative path from them can result in the same URL.
                    if datum["sym"] in handled_syms:
                        continue
                    handled_syms.add(datum["sym"])

                    lines.append(json.dumps(datum))


            if sym == "RELPATH":
                # This is special record for relative path import.
                # Resolve it based on the URLs for the current file,
                # and then map it to the corresponding files.
                relpath = datum["pretty"]

                if path not in reverse_map:
                    continue

                for url in reverse_map[path]:
                    new_url = resolve_relpath(url, relpath)
                    if new_url is None:
                        continue
                    new_sym = "URL_" + at_escape(new_url)
                    handle_url_sym(new_sym)
            else:
                handle_url_sym(sym)

    if not has_alias:
        return

    with open(fullpath, "w") as f:
        for line in lines:
            print(line, file=f)


with open(alias_map_path, "r") as f:
    alias_map = json.load(f)

reverse_map = {}
for sym, aliases in alias_map.items():
    for item in aliases:
        path = item["pretty"].replace("file ", "")
        url = at_unescape(sym.replace("URL_", ""))

        if path not in reverse_map:
            reverse_map[path] = []

        reverse_map[path].append(url)

for path in sys.stdin:
    path = path.rstrip()
    if not path:
        continue

    replace_aliases(path, alias_map, reverse_map)
