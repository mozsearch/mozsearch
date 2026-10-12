#!/usr/bin/env python3
"""Generate a tree of web specs' specs.toml (ex: the wubspecs tree's; see
tools/src/spec_forest.rs and tools/src/spec_analyze) from browser-specs' index
(https://github.com/w3c/browser-specs) and a file of overrides.

Usage: browser-specs-to-toml.py OVERRIDES.toml OUTPUT.toml [--index INDEX.json]

The specs are browser-specs' whose URLs are on the hosts in HOSTS, in the
formats spec-analyze analyzes (see `spec_format`): Bikeshed (`.bs` and
`.src.html` sources), ecmarkup (tc39.es's specs), and Wattsi (the HTML
Standard).  Each upstream repository is a `[[spec]]` with a `url` (a
directory of the forest, named by OVERRIDES' `directories`, else the
repository's name, lowercased), and a repository with several specs (ex:
csswg-drafts) has a `[[spec]]` without a `url` for each, by its shortname,
with its `path` in the forest (or, if that's another repository's directory,
by its repository's and its shortnames).  A spec's symbols are by its series'
shortname if it's its series' current specification (with the series'
unversioned URL as an alias, ex: "https://drafts.csswg.org/css-grid/" for
css-grid-2), else by its own (ex: css-grid-3, whose `series` is css-grid).
Its `successors` are the specs browser-specs says obsolete it (ex: html for
sanitizer-api), which are configured.  (Citations of anchors a spec doesn't
define are of its series' or successors', if they define them.)

OVERRIDES has the forest's root `.gitattributes` (`attributes`), the
`directories` of repositories by their URLs, and repositories and specs to
leave out (`exclude`, by URL or shortname).
"""

import argparse
import json
import re
import sys
import tomllib
import urllib.request

INDEX_URL = "https://w3c.github.io/browser-specs/index.json"

# The hosts of the specs of the 2025 plan (see asuth-gecko-notes'
# wubspecs-design.md).
HOSTS = re.compile(
    r"^https://(w3c\.github\.io|[a-z0-9-]+\.spec\.whatwg\.org|drafts\.csswg\.org"
    r"|wicg\.github\.io|tc39\.es)/"
)

# Each format's default source, which specs.toml leaves out (see
# `SpecEntry::source_path`).
DEFAULT_SOURCES = {"wattsi": "source", "ecmarkup": "spec.html", "bikeshed": "index.bs"}


def spec_url(spec):
    return (spec.get("nightly") or {}).get("url") or spec["url"]


def spec_format(spec):
    """The format spec-analyze analyzes the spec's source in, if any."""
    source = (spec.get("nightly") or {}).get("sourcePath") or ""
    if spec_url(spec).startswith("https://tc39.es/") and source.endswith((".html", ".emu")):
        return "ecmarkup"
    if source == "source":
        return "wattsi"
    if source.endswith((".bs", ".src.html")):
        return "bikeshed"
    # (ReSpec specs, for now.)
    return None


def rendered_url(url):
    """The spec's single-page URL, where its anchors are (ex: the HTML
    Standard's, rather than its multipage edition's)."""
    return re.sub(r"multipage/$", "", url)


def symbol_shortname(spec):
    """The shortname of the spec's symbols: its series' if it's its series'
    current specification, else its own."""
    series = spec.get("series") or {}
    current = series.get("currentSpecification") in (None, spec["shortname"])
    return series.get("shortname", spec["shortname"]) if current else spec["shortname"]


def toml_string(value):
    return json.dumps(value, ensure_ascii=False)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("overrides")
    parser.add_argument("output")
    parser.add_argument("--index", default=INDEX_URL, help="browser-specs' index.json (a path or URL)")
    args = parser.parse_args()

    with open(args.overrides, "rb") as f:
        overrides = tomllib.load(f)
    if re.match(r"^https?://", args.index):
        with urllib.request.urlopen(args.index) as response:
            index = json.load(response)
    else:
        with open(args.index) as f:
            index = json.load(f)
    directories = overrides.get("directories", {})
    excluded = set(overrides.get("exclude", []))

    # The specs we analyze, by their repositories.
    repos = {}
    for spec in index:
        repo = ((spec.get("nightly") or {}).get("repository") or "").rstrip("/")
        if not repo or not HOSTS.match(spec_url(spec)) or spec_format(spec) is None:
            continue
        if repo in excluded or spec["shortname"] in excluded:
            continue
        repos.setdefault(repo, []).append(spec)

    dirs = {}
    for repo in repos:
        name = directories.get(repo) or repo.rsplit("/", 1)[-1].lower()
        if name in dirs:
            sys.exit(f"{repo} and {dirs[name]} would both be {name}/; give one a directory")
        dirs[name] = repo

    # The configured specs' symbols' shortnames, by their browser-specs
    # shortnames, for specs' successors.
    shortnames = {spec["shortname"]: symbol_shortname(spec) for specs in repos.values() for spec in specs}

    out = []
    out.append(
        "# The specs of the \"wubspecs\" tree, generated by mozsearch's\n"
        "# scripts/browser-specs-to-toml.py from browser-specs' index and\n"
        "# specs-overrides.toml (which to edit rather than this).  Each spec with a\n"
        "# `url` is a top-level directory of the tree's forest repository, with the\n"
        "# history of its upstream repository (see tools/src/spec_forest.rs and\n"
        "# build-spec-forest), and those without are specs in another's repository.\n"
        "#\n"
        "# Each spec's `shortname` (if it's not its name; as browser-specs' series\n"
        "# shortnames for current specifications) names its symbols\n"
        "# (`SPEC_<shortname>_<anchor>`), its `rendered` URL (and `aliases`) is where\n"
        "# its anchors are, so that references to it from other specs (and from code)\n"
        "# name its symbols, and its `format` says how spec-analyze analyzes its\n"
        "# source (see tools/src/spec_analyze).  Citations of anchors it doesn't\n"
        "# define are of the other specs of its `series` (if it's not its shortname)\n"
        "# or of its `successors`, if they define them.\n"
    )
    out.append(f"attributes = \"\"\"\n{overrides.get('attributes', '').strip()}\n\"\"\"\n")
    names = set()

    def add_spec(spec, name, path, url=None):
        if name in names:
            sys.exit(f"{name} is configured twice")
        names.add(name)
        series = spec.get("series") or {}
        current = series.get("currentSpecification") in (None, spec["shortname"])
        shortname = symbol_shortname(spec)
        fmt = spec_format(spec)
        source = spec["nightly"].get("sourcePath")
        rendered = rendered_url(spec_url(spec))
        aliases = []
        if current and series.get("nightlyUrl"):
            alias = rendered_url(series["nightlyUrl"])
            if alias != rendered:
                aliases.append(alias)
        lines = ["[[spec]]", f"name = {toml_string(name)}"]
        if url:
            lines.append(f"url = {toml_string(url + '.git')}")
        if path:
            lines.append(f"path = {toml_string(path)}")
        if shortname != name:
            lines.append(f"shortname = {toml_string(shortname)}")
        lines.append(f"format = {toml_string(fmt)}")
        if source != DEFAULT_SOURCES[fmt]:
            lines.append(f"source = {toml_string(source)}")
        lines.append(f"rendered = {toml_string(rendered)}")
        if aliases:
            lines.append(f"aliases = [{', '.join(toml_string(a) for a in aliases)}]")
        if series.get("shortname", shortname) != shortname:
            lines.append(f"series = {toml_string(series['shortname'])}")
        successors = [shortnames[s] for s in spec.get("obsoletedBy") or [] if s in shortnames]
        if successors:
            lines.append(f"successors = [{', '.join(toml_string(s) for s in successors)}]")
        out.append("\n".join(lines) + "\n")

    for name in sorted(dirs):
        repo = dirs[name]
        specs = sorted(repos[repo], key=lambda s: s["shortname"])
        # (A spec named like its repository's directory, or its only one, is
        # the repository's `[[spec]]`.)
        own = [s for s in specs if s["shortname"] == name] or (specs if len(specs) == 1 else [])
        if own:
            add_spec(own[0], name, None, url=repo)
        else:
            if name in names:
                sys.exit(f"{name} is configured twice")
            names.add(name)
            out.append(f"[[spec]]\nname = {toml_string(name)}\nurl = {toml_string(repo + '.git')}\n")
        for spec in specs:
            if own and spec is own[0]:
                continue
            # (A spec named like another repository's directory, ex: a
            # repository's "bluetooth" spec and the bluetooth repository,
            # is named by its own repository too.)
            spec_name = spec["shortname"]
            if spec_name in dirs or spec_name in names:
                spec_name = f"{name}-{spec_name}"
            add_spec(spec, spec_name, name)

    with open(args.output, "w") as f:
        f.write("\n".join(out))
    print(
        f"{len(dirs)} repositories, {sum(len(s) for s in repos.values())} specs",
        file=sys.stderr,
    )


if __name__ == "__main__":
    main()
