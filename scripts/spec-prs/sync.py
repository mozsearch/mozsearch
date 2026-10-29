#!/usr/bin/env python3
"""Bring a tree of specs' pull request summaries up to date (see README.md):
for each configured spec repository (a forest directory), its open pull
requests, from a snapshot of their metadata (see snapshot.py), and what each
changes relative to the spec's trunk in the forest: its heads are fetched
through git (GitHub's `refs/pull/N/head`, into `refs/upstream-pr/SPEC/N`, as
build-spec-pr does), and diffed from their merge bases with the trunk, for
their files' hunks, the specs they're in (and which files are their sources),
and the sections they change (the headings before their hunks, in the merge
base's versions), and their commits (with their patch ids and their messages'
hashes, for finding the pull requests which have others' commits, ex: stacked
on them; see tools/src/spec_prs.rs's `containments`).  Not a history:
only the pull requests' current heads, written to OUT_DIR/SPEC.json, for the
web server's pull request pages (see tools/src/spec_prs.rs).  A pull request
whose head and trunk haven't changed keeps its summary.

Usage: sync.py --specs SPECS_TOML --config SPEC_PRS_TOML --forest FOREST_REPO
               --out OUT_DIR [--spec SPEC]

SPEC_PRS_TOML has `repos`, the forest directories whose pull requests are
summarized (all of them, or --spec's), and `snapshot`, the snapshot's URL or
path (relative to SPEC_PRS_TOML's directory), if any.  A repository the
snapshot doesn't have (or has incompletely), or all of them without one, gets
its metadata from GitHub's REST API (see snapshot.py), unauthenticated, but
only with --spec (60 requests an hour wouldn't do for all of them), and its
review decisions only if it has at most MAX_REST_REVIEWS open pull requests.
"""

import argparse
import datetime
import hashlib
import json
import os
import re
import subprocess
import sys
import time
import tomllib
import urllib.request

# (Without writing its bytecode into the mozsearch checkout.)
sys.dont_write_bytecode = True
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import snapshot as snapshots  # noqa: E402

# The most pull requests whose reviews a sync without a snapshot asks the
# REST API for, unauthenticated.
MAX_REST_REVIEWS = 40

HUNK = re.compile(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@")
HEADING = re.compile(r"<h([1-6])\b[^>]*?\bid=[\"']?([^\"'\s>]+)[^>]*>(.*?)(?:</h\1>|$)", re.I)
CLAUSE = re.compile(r"<emu-clause\b[^>]*?\bid=[\"']?([^\"'\s>]+)", re.I)
CLAUSE_TITLE = re.compile(r"<h1>(.*?)</h1>", re.I)
MARKDOWN = re.compile(r"^(#{1,6})\s+(.*?)\s*(?:\{#([^}\s]+)\})?\s*$")
# ecmarkup's imports (as tools/src/spec_analyze/ecmarkup.rs's `imports`).
EMU_IMPORT = re.compile(r"""<emu-import\s+href=["']?([^"'\s>]+)["']?\s*(?:/>|>\s*</emu-import>)""")
# The version of the pull requests' summaries' format: older ones are made
# again (ex: without their files' specs for the files ecmarkup specs import,
# or their commits).
SUMMARY_VERSION = 3
# The most commits of a pull request which its summary lists (see `commits`).
MAX_COMMITS = 250
TAG = re.compile(r"<[^>]+>")
# Bikeshed's autolinks in headings (ex: "Interface {{Node}}").
AUTOLINK = re.compile(r"\{\{(.*?)\}\}|\[=(.*?)=\]|\[\[(.*?)\]\]|<<(.*?)>>|''(.*?)''")


def log(message):
    print(message, flush=True)


def git(forest, *args, check=True):
    result = subprocess.run(["git", "-C", forest, *args], capture_output=True, text=True,
                            errors="replace")
    if check and result.returncode != 0:
        raise Exception(f"git {' '.join(args[:3])}: {result.stderr.strip()}")
    return result.stdout


def sections(text):
    """The headings of a spec's source, as (line, id, title), in order."""
    found = []
    lines = text.split("\n")
    for i, line in enumerate(lines):
        title = None
        match = HEADING.search(line)
        if match:
            ident, title = match.group(2), match.group(3)
        elif match := CLAUSE.search(line):
            ident = match.group(1)
            following = " ".join(lines[i:i + 3])
            title_match = CLAUSE_TITLE.search(following)
            title = title_match.group(1) if title_match else ident
        elif match := MARKDOWN.match(line):
            ident, title = match.group(3), match.group(2)
        else:
            continue
        title = TAG.sub("", title or "")
        title = AUTOLINK.sub(lambda m: next(g for g in m.groups() if g is not None), title).strip()
        found.append((i + 1, ident, title))
    return found


def imported_files(forest, spec_dir, sources, trunk):
    """The files that the repository's ecmarkup specs' sources import (and
    that those import), at the trunk, by their paths in the forest, with their
    specs' names."""
    imported = {}
    for name, source, format in sources:
        if format != "ecmarkup":
            continue
        queue = [source]
        while queue and len(imported) < 1000:
            path = queue.pop()
            try:
                text = git(forest, "show", f"{trunk}:{path[len(spec_dir) + 1:]}")
            except Exception:
                continue
            for match in EMU_IMPORT.finditer(text):
                target = os.path.normpath(os.path.join(os.path.dirname(path), match.group(1)))
                if target != source and target not in imported:
                    imported[target] = name
                    queue.append(target)
    return imported


def commits(forest, merge_base, head):
    """A pull request's commits since its merge base with the trunk, oldest
    first: each's hash, its patch id (`git patch-id --stable`, which its
    versions with the same changes have too, ex: rebased; none for a merge),
    the hashes of its message (with its whitespace collapsed, which its
    versions with other changes, ex: amended, may have too) and of its
    author's email, and its subject, or None if it has more than
    MAX_COMMITS."""
    span = f"{merge_base}..{head}"
    if int(git(forest, "rev-list", "--count", span).strip() or 0) > MAX_COMMITS:
        return None
    patches = git(forest, "log", "--reverse", "-p", "--no-merges", "--no-color",
                  "--no-ext-diff", "--format=commit %H", span)
    result = subprocess.run(["git", "-C", forest, "patch-id", "--stable"], input=patches,
                            capture_output=True, text=True, errors="replace")
    patch_ids = {}
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) == 2:
            patch_ids[parts[1]] = parts[0]
    found = []
    digest = lambda text: hashlib.sha1(text.encode()).hexdigest()[:16]
    for record in git(forest, "log", "--reverse", "--format=%H%x00%ae%x00%B%x01",
                      span).split("\x01"):
        record = record.lstrip("\n")
        if not record:
            continue
        sha, author, message = record.split("\x00", 2)
        found.append({
            "sha": sha,
            "patch_id": patch_ids.get(sha),
            "message": digest(" ".join(message.split())),
            "author": digest(author.lower()),
            "subject": message.strip().split("\n")[0][:200],
        })
    return found


def summarize(forest, spec_dir, sources, imported, number, head, trunk):
    """A pull request's changes from its merge base with the trunk (upstream
    commits, whose paths are relative to the repository's directory in the
    forest, which the summary's paths are prefixed with).  Its files which are
    specs' sources, or which ecmarkup specs import (`imported`), have their
    specs."""
    merge_base = git(forest, "merge-base", trunk, head).strip()
    diff = git(forest, "diff", "-U0", "--no-color", "--no-renames", "--no-ext-diff",
               merge_base, head)
    files, current = [], None
    for line in diff.split("\n"):
        if line.startswith("diff --git "):
            current = None
        elif line.startswith("--- "):
            old = line[4:]
        elif line.startswith("+++ "):
            new = line[4:]
            path = (new if new != "/dev/null" else old)[2:]
            current = {"path": f"{spec_dir}/{path}", "upstream_path": path,
                       "added": 0, "removed": 0, "hunks": [], "sections": []}
            files.append(current)
        elif current is not None and (match := HUNK.match(line)):
            b, bl, h, hl = (int(match.group(1)), int(match.group(2) or 1),
                            int(match.group(3)), int(match.group(4) or 1))
            current["hunks"].append([b, bl, h, hl])
        elif current is not None and line.startswith("+"):
            current["added"] += 1
        elif current is not None and line.startswith("-"):
            current["removed"] += 1
    specs = set()
    for file in files:
        for name, source, _ in sources:
            directory = os.path.dirname(source)
            if file["path"].startswith(directory + "/"):
                specs.add(name)
            if file["path"] == source or imported.get(file["path"]) == name:
                file["spec"] = name
        # (The sections before its hunks, in the merge base's version.)
        try:
            base_text = git(forest, "show", f"{merge_base}:{file.pop('upstream_path')}",
                            check=True)
        except Exception:
            file.pop("upstream_path", None)
            continue
        headings = sections(base_text)
        seen = set()
        for b, _, _, _ in file["hunks"]:
            before = [heading for heading in headings if heading[0] <= max(b, 1)]
            if before:
                line, ident, title = before[-1]
                if ident not in seen:
                    seen.add(ident)
                    file["sections"].append({"line": line, "id": ident, "title": title})
    return {"merge_base": merge_base, "files": files, "specs": sorted(specs),
            "commits": commits(forest, merge_base, head)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--specs", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--forest", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--spec")
    args = parser.parse_args()
    with open(args.specs, "rb") as f:
        specs = tomllib.load(f).get("spec", [])
    with open(args.config, "rb") as f:
        config = tomllib.load(f)
    urls = {spec["name"]: spec["url"] for spec in specs if spec.get("url")}
    defaults = {"wattsi": "source", "ecmarkup": "spec.html", "bikeshed": "index.bs",
                "respec": "index.html"}
    # The specs' sources by their repositories' directories.
    sources = {}
    for spec in specs:
        if not spec.get("format"):
            continue
        directory = spec.get("path", spec["name"])
        source = spec.get("source", defaults.get(spec["format"], ""))
        sources.setdefault(directory.split("/")[0], []).append(
            (spec["name"], f"{directory}/{source}", spec["format"]))
    repos = [repo for repo in config.get("repos", []) if repo in urls]
    if args.spec:
        repos = [repo for repo in repos if repo == args.spec]
        if not repos:
            log(f"ERROR: {args.spec} isn't a configured spec repository")
            sys.exit(1)

    snapshot = {"repos": {}, "taken": None}
    location = config.get("snapshot")
    if location:
        log(f"Reading the snapshot of the pull requests' metadata, {location}")
        try:
            if re.match(r"https?://", location):
                request = urllib.request.Request(location,
                                                 headers={"User-Agent": snapshots.USER_AGENT})
                with urllib.request.urlopen(request, timeout=120) as response:
                    snapshot = json.loads(response.read())
            else:
                path = os.path.join(os.path.dirname(os.path.abspath(args.config)), location)
                with open(path) as f:
                    snapshot = json.load(f)
        except Exception as e:
            # (Then only --spec's repository, from the API.)
            log(f"WARNING: The snapshot couldn't be read: {e}")
    os.makedirs(args.out, exist_ok=True)
    now = datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    for count, repo in enumerate(repos):
        if count:
            # (Politely, one repository at a time.)
            time.sleep(1)
        key = snapshots.github_repo(urls[repo]) or repo
        entry = snapshot.get("repos", {}).get(key)
        taken = snapshot.get("taken")
        if not entry or entry.get("incomplete"):
            if not (args.spec and snapshots.github_repo(urls[repo])):
                log(f"{repo}: not in the snapshot; skipped")
                continue
            log(f"{repo}: asking GitHub's API for its open pull requests")
            client = snapshots.Client(os.environ.get("GITHUB_TOKEN"), 600)
            owner, name = key.split("/")
            try:
                if client.token:
                    prs = snapshots.graphql_prs(client, owner, name)
                else:
                    prs = snapshots.rest_prs(client, owner, name, False)
                    # (Their reviews are a request each, of the 60 an hour.)
                    if len(prs) <= MAX_REST_REVIEWS:
                        snapshots.rest_reviews(client, owner, name, prs)
                    else:
                        log(f"{repo}: {len(prs)} pull requests, so without their review "
                            f"decisions (at most {MAX_REST_REVIEWS} without a snapshot)")
            except snapshots.RateLimited as e:
                log(f"ERROR: {repo}: {e}")
                sys.exit(1)
            entry, taken = {"prs": prs}, now
        prs = entry["prs"]
        out_path = os.path.join(args.out, f"{repo}.json")
        previous = {}
        if os.path.exists(out_path):
            with open(out_path) as f:
                previous = {pr["number"]: pr for pr in json.load(f).get("prs", [])}
        trunk = git(args.forest, "rev-parse", f"refs/forest/{repo}").strip()
        imported = imported_files(args.forest, repo, sources.get(repo, []), trunk)

        # Their heads, through git, in batches.
        log(f"{repo}: fetching the heads of its {len(prs)} open pull requests")
        for start in range(0, len(prs), 100):
            refspecs = [f"+refs/pull/{pr['number']}/head:refs/upstream-pr/{repo}/{pr['number']}"
                        for pr in prs[start:start + 100]]
            if refspecs:
                git(args.forest, "fetch", "--quiet", "--no-tags", urls[repo], *refspecs,
                    check=False)

        summaries = []
        for pr in prs:
            summary = dict(pr)
            old = previous.get(pr["number"])
            fetched = git(args.forest, "rev-parse", "--verify", "--quiet",
                          f"refs/upstream-pr/{repo}/{pr['number']}^{{commit}}",
                          check=False).strip()
            # (Its head is what git fetched, which may be newer than the
            # snapshot's.)
            if not fetched:
                summary["error"] = "Its head couldn't be fetched"
            elif fetched != pr.get("head"):
                if pr.get("head"):
                    summary["snapshot_head"] = pr["head"]
                summary["head"] = fetched
            if not fetched:
                pass
            elif old and old.get("head") == fetched and old.get("trunk") == trunk \
                    and "files" in old and old.get("version") == SUMMARY_VERSION:
                summary.update({k: old[k] for k in ("merge_base", "files", "specs", "commits")})
            else:
                try:
                    summary.update(summarize(args.forest, repo, sources.get(repo, []), imported,
                                             pr["number"], fetched, trunk))
                except Exception as e:
                    summary["error"] = str(e)
            summary["trunk"] = trunk
            summary["version"] = SUMMARY_VERSION
            summaries.append(summary)
        summaries.sort(key=lambda pr: pr["number"], reverse=True)
        result = {"spec": repo, "repo": key, "url": urls[repo], "synced": now, "snapshot": taken,
                  "trunk": trunk, "prs": summaries}
        tmp = out_path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(result, f, indent=1)
            f.write("\n")
        os.replace(tmp, out_path)
        log(f"{repo}: {len(summaries)} pull requests summarized")
    log("Done")


if __name__ == "__main__":
    main()
