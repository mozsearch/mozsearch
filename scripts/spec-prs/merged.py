#!/usr/bin/env python3
"""Write a snapshot of a tree of specs' repositories' merged pull requests,
for the blame popups' links to a commit's pull request and the issues it
closes (see tools/src/pull_requests.rs and README.md): for each GitHub
repository (OWNER/NAME) of the specs in SPECS_TOML, the specs whose
repository it is, and its merged pull requests, each its number, its merge
commit (for a squash merge, the squashed commit) and the issues it closes
(GitHub's closing references: "#N" in the repository, or "OWNER/NAME#N").

Usage: merged.py --specs SPECS_TOML --out MERGED_JSON [--previous PREVIOUS]
                 [--max-wait SECONDS] [--budget SECONDS] [REPO...]

With the PREVIOUS snapshot (a path or URL; ex: the last one published), it
only asks for the pull requests updated since each repository's newest, and
continues the repositories it didn't finish: the pull requests are asked for
newest first (by when they were updated), 100 at a time, from where it left
off (`cursor`).  The repositories it only needs a page of are asked for 20 at
a time, their 25 newest each.  It stops after --budget seconds (900 by
default), when GitHub's rate limit would make it wait more than --max-wait
seconds (600 by default; see snapshot.py), or when fewer than --reserve of
the limit's points are left (200 by default, for the open pull requests'
snapshot), keeping where it was.
With REPO arguments, only those repositories are asked for (the others are as
they were).

It needs a token in GITHUB_TOKEN (ex: a GitHub Action's), since GitHub's
GraphQL API does.  The snapshot is about 60 bytes per pull request.
"""

import argparse
import datetime
import json
import os
import sys
import time
import tomllib
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from snapshot import API, USER_AGENT, Client, RateLimited, github_repo  # noqa: E402

FIELDS = """
pageInfo { hasNextPage endCursor }
nodes {
  number updatedAt
  mergeCommit { oid }
  closingIssuesReferences(first: 10) { nodes { number repository { nameWithOwner } } }
}
"""

PAGE_QUERY = """
query($owner: String!, $name: String!, $after: String) {
  rateLimit { remaining }
  repository(owner: $owner, name: $name) {
    pullRequests(states: MERGED, first: 100, after: $after,
                 orderBy: {field: UPDATED_AT, direction: DESC}) { %s }
  }
}
""" % FIELDS

BATCH = 20
BATCH_FIRST = 25


def spec_repos(specs_toml):
    """The GitHub repositories of a tree's specs, and each one's specs."""
    with open(specs_toml, "rb") as f:
        specs = tomllib.load(f).get("spec", [])
    repos = {}
    for spec in specs:
        repo = github_repo(spec.get("url"))
        if repo:
            repos.setdefault(repo, []).append(spec["name"])
    return repos


def read_previous(previous):
    if not previous:
        return {}
    try:
        if previous.startswith("https://"):
            request = urllib.request.Request(previous, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(request, timeout=120) as response:
                data = json.loads(response.read())
        else:
            with open(previous) as f:
                data = json.load(f)
    except Exception as e:
        print(f"No previous snapshot ({previous}: {e}); starting over", file=sys.stderr)
        return {}
    return data.get("repos", {})


def closes(node, repo):
    refs = []
    for issue in node["closingIssuesReferences"]["nodes"]:
        other = issue["repository"]["nameWithOwner"]
        refs.append(f"#{issue['number']}" if other == repo else f"{other}#{issue['number']}")
    return refs


class Repo:
    """A repository's pull requests (by number), the newest update time it has,
    and where its pass through the older ones is (None when it's done)."""

    def __init__(self, name, specs, previous):
        self.name = name
        self.specs = specs
        self.prs = {pr[0]: pr for pr in (previous or {}).get("prs", [])}
        self.newest = (previous or {}).get("newest")
        # (A new repository starts from the newest.)
        self.cursor = (previous or {}).get("cursor", "" if previous is None else None)
        self.front_newest = None

    def add(self, nodes):
        """Add a page's pull requests; returns whether they're all newer than
        the newest of before (so the next page may be too)."""
        newer = True
        for node in nodes:
            if self.newest and node["updatedAt"] <= self.newest:
                newer = False
                continue
            merge = (node.get("mergeCommit") or {}).get("oid")
            self.prs[node["number"]] = [node["number"], merge, closes(node, self.name)]
            if not self.front_newest or node["updatedAt"] > self.front_newest:
                self.front_newest = node["updatedAt"]
        return newer

    def json(self):
        return {
            "specs": self.specs,
            "newest": max(filter(None, [self.newest, self.front_newest]), default=None),
            "cursor": self.cursor,
            "prs": [self.prs[number] for number in sorted(self.prs)],
        }


# The points of GraphQL's hourly limit to leave for others (ex: the open pull
# requests' snapshot, whose next run may come before the limit resets).
RESERVE = 200


def graphql(client, query, variables=None):
    result = client.request(f"{API}/graphql", {"query": query, "variables": variables or {}})
    if result.get("errors"):
        raise Exception(f"GraphQL errors: {result['errors']}")
    # (Its results are kept, but it's the last; see `main`'s `stopping`.)
    remaining = (result["data"].get("rateLimit") or {}).get("remaining")
    if remaining is not None and remaining < getattr(client, "reserve", RESERVE):
        client.exhausted = f"only {remaining} points of the hour's rate limit are left"
    return result["data"]


def page(client, repo, after):
    owner, name = repo.name.split("/", 1)
    data = graphql(client, PAGE_QUERY, {"owner": owner, "name": name, "after": after})
    if not data.get("repository"):
        raise Exception(f"{repo.name}: no such repository")
    return data["repository"]["pullRequests"]


def front_pass_batch(client, repos):
    """The newest pull requests of several repositories, in one query; returns
    those which need more pages."""
    parts = []
    for i, repo in enumerate(repos):
        owner, name = repo.name.split("/", 1)
        parts.append(
            f"r{i}: repository(owner: {json.dumps(owner)}, name: {json.dumps(name)}) {{ "
            f"pullRequests(states: MERGED, first: {BATCH_FIRST}, "
            f"orderBy: {{field: UPDATED_AT, direction: DESC}}) {{ {FIELDS} }} }}")
    data = graphql(client, "query {\nrateLimit { remaining }\n" + "\n".join(parts) + "\n}")
    more = []
    for i, repo in enumerate(repos):
        found = data.get(f"r{i}")
        if not found:
            print(f"{repo.name}: no such repository", file=sys.stderr)
            continue
        connection = found["pullRequests"]
        if repo.add(connection["nodes"]) and connection["pageInfo"]["hasNextPage"]:
            more.append((repo, connection["pageInfo"]["endCursor"]))
    return more


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--specs", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--previous")
    parser.add_argument("--max-wait", type=int, default=600)
    parser.add_argument("--budget", type=int, default=900)
    parser.add_argument("--reserve", type=int, default=RESERVE)
    parser.add_argument("repos", nargs="*")
    args = parser.parse_args()
    token = os.environ.get("GITHUB_TOKEN")
    if not token:
        parser.error("GitHub's GraphQL API needs a token in GITHUB_TOKEN")
    started = time.time()
    client = Client(token, args.max_wait)
    client.reserve = args.reserve
    previous = read_previous(args.previous)
    configured = spec_repos(args.specs)
    repos = {name: Repo(name, specs, previous.get(name)) for name, specs in sorted(configured.items())}
    asked = [repos[name] for name in (args.repos or repos) if name in repos]
    limited = None

    def stopping():
        return time.time() - started > args.budget or getattr(client, "exhausted", None)

    try:
        # The pull requests updated since each repository's newest: those of
        # the repositories which have one, 20 repositories at a time, and
        # then the others' pages, until they're older than the newest.
        front = [repo for repo in asked if repo.newest]
        more = []
        for i in range(0, len(front), BATCH):
            if stopping():
                break
            more += front_pass_batch(client, front[i:i + BATCH])
        for repo, after in more:
            while after and not stopping():
                connection = page(client, repo, after)
                after = connection["pageInfo"]["endCursor"] if (
                    repo.add(connection["nodes"]) and connection["pageInfo"]["hasNextPage"]) else None
        # The repositories' older pull requests, from where each left off.
        for repo in asked:
            while repo.cursor is not None and not stopping():
                connection = page(client, repo, repo.cursor or None)
                # (All of them, since they're older than the newest when it
                # started; see `Repo::add`.)
                newest, repo.newest = repo.newest, None
                repo.add(connection["nodes"])
                repo.newest = newest or repo.front_newest
                repo.cursor = connection["pageInfo"]["endCursor"] if connection["pageInfo"]["hasNextPage"] else None
            if repo.cursor is None:
                print(f"{repo.name}: {len(repo.prs)} merged pull requests", file=sys.stderr, flush=True)
    except RateLimited as e:
        limited = str(e)
        print(f"Stopping: {e}", file=sys.stderr, flush=True)

    snapshot = {
        "taken": datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "repos": {name: repo.json() for name, repo in repos.items()},
        "requests": client.requests,
        "unfinished": sorted(name for name, repo in repos.items() if repo.cursor is not None),
    }
    limited = limited or getattr(client, "exhausted", None)
    if limited:
        snapshot["limited"] = limited
    tmp = args.out + ".tmp"
    with open(tmp, "w") as f:
        json.dump(snapshot, f, separators=(",", ":"), sort_keys=True)
        f.write("\n")
    os.replace(tmp, args.out)
    print(f"{len(repos)} repositories, {sum(len(r.prs) for r in repos.values())} merged pull requests, "
          f"{client.requests} requests, {len(snapshot['unfinished'])} unfinished", file=sys.stderr)


if __name__ == "__main__":
    main()
