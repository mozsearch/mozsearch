#!/usr/bin/env python3
"""Archive hg.mozilla.org's pushlogs (the json-pushes command of
version-control-tools' hgext/pushlog) as gzipped JSON chunks of 1,000 pushes,
for the hg-era landing dates of searchfox's history (see asuth-gecko-notes
tooling/searchfox/blame-evolution-notes.md, "hg pushlog").

Usage: scrape_pushlog.py OUT_DIR [REPO...]

Resumable: chunks already in OUT_DIR are skipped.  hg.mozilla.org's WAF
answers 429 when it rate-limits, with a cool-off of about 10 minutes, so a
429 waits at least that long (or its Retry-After) and doubles the pause
between requests.
"""

import gzip
import json
import os
import random
import sys
import time
import urllib.error
import urllib.request

BASE = "https://hg-edge.mozilla.org"
USER_AGENT = "searchfox-pushlog-archive/1.0 (+https://github.com/mozsearch/mozsearch)"
CHUNK = 1000
COOL_OFF = 600
MAX_PAUSE = 60.0

# mozilla-central and the integration repos (where changes landed), then the
# release repos (uplifts), which only branch histories would need.  (The old
# project branches, ex: tracemonkey, projects/ionmonkey, are gone; their
# changesets' first pushes are their merges into these.)
DEFAULT_REPOS = [
    "mozilla-central",
    "integration/mozilla-inbound",
    "integration/autoland",
    "integration/fx-team",
    "integration/b2g-inbound",
    "services/services-central",
    "releases/mozilla-beta",
    "releases/mozilla-release",
    "releases/mozilla-aurora",
] + [
    f"releases/mozilla-esr{n}"
    for n in (10, 17, 24, 31, 38, 45, 52, 60, 68, 78, 91, 102, 115, 128, 140)
]

pause = 3.0


def log(out_dir, message):
    line = f"{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())} {message}"
    print(line, flush=True)
    with open(os.path.join(out_dir, "scrape.log"), "a") as f:
        f.write(line + "\n")


def fetch(out_dir, repo, start, end):
    """The pushes with IDs in (start, end], as JSON text, or None if the repo
    doesn't exist."""
    global pause
    url = f"{BASE}/{repo}/json-pushes?version=2&startID={start}&endID={end}"
    failures = 0
    while True:
        request = urllib.request.Request(
            url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"}
        )
        try:
            with urllib.request.urlopen(request, timeout=300) as response:
                return response.read()
        except urllib.error.HTTPError as e:
            if e.code == 429:
                retry_after = e.headers.get("Retry-After", "")
                wait = max(COOL_OFF, int(retry_after) if retry_after.isdigit() else 0)
                wait += random.randint(30, 90)
                pause = min(pause * 2, MAX_PAUSE)
                log(out_dir, f"429 for {url}; waiting {wait} s, then {pause} s between requests")
                time.sleep(wait)
                continue
            if e.code == 404:
                return None
            if e.code >= 500 and failures < 10:
                failures += 1
                log(out_dir, f"HTTP {e.code} for {url}; retrying in {60 * failures} s")
                time.sleep(60 * failures)
                continue
            raise
        except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
            if failures < 10:
                failures += 1
                log(out_dir, f"{e!r} for {url}; retrying in {60 * failures} s")
                time.sleep(60 * failures)
                continue
            raise


def scrape(out_dir, repo):
    repo_dir = os.path.join(out_dir, repo.replace("/", "_"))
    os.makedirs(repo_dir, exist_ok=True)
    # (The last push's ID, from a request for the first push.)
    first = fetch(out_dir, repo, 0, 1)
    time.sleep(pause)
    if first is None:
        log(out_dir, f"{repo}: no such repo")
        return
    last = json.loads(first)["lastpushid"]
    with open(os.path.join(repo_dir, "lastpushid"), "w") as f:
        f.write(f"{last}\n")
    log(out_dir, f"{repo}: {last} pushes")
    for start in range(0, last, CHUNK):
        end = min(start + CHUNK, last)
        path = os.path.join(repo_dir, f"{start + 1:07d}-{end:07d}.json.gz")
        if os.path.exists(path):
            continue
        body = fetch(out_dir, repo, start, end)
        pushes = json.loads(body)["pushes"]
        ids = sorted(int(i) for i in pushes)
        if ids and (ids[0] <= start or ids[-1] > end):
            raise Exception(f"{repo}: asked for ({start}, {end}], got {ids[0]}..{ids[-1]}")
        tmp = path + ".tmp"
        with gzip.open(tmp, "wb") as f:
            f.write(body)
        os.rename(tmp, path)
        changesets = sum(len(push["changesets"]) for push in pushes.values())
        log(out_dir, f"{repo}: {start + 1}-{end}: {len(pushes)} pushes, {changesets} changesets")
        time.sleep(pause)


def main():
    out_dir = sys.argv[1]
    os.makedirs(out_dir, exist_ok=True)
    for repo in sys.argv[2:] or DEFAULT_REPOS:
        scrape(out_dir, repo)
    log(out_dir, "done")


if __name__ == "__main__":
    main()
