#!/usr/bin/env python3
"""Write a snapshot of spec repositories' open pull requests' metadata, which
a tree of specs' pull request summaries are made from (see sync.py and
README.md): for each repository (OWNER/NAME), its open pull requests' numbers,
titles, pages, authors and their associations with the repository, drafts,
labels, review decisions, heads, base branches and update times.

Usage: snapshot.py --out SNAPSHOT_JSON [--no-reviews] [--max-wait SECONDS]
                   [--specs SPECS_TOML --config SPEC_PRS_TOML] [REPO...]

The repositories are the REPO arguments, and with --specs and --config, the
GitHub repositories of a tree of specs' configured spec repositories (as
sync.py reads them: SPEC_PRS_TOML's `repos`, by their URLs in SPECS_TOML).

With a token in GITHUB_TOKEN (ex: a GitHub Action's), it asks GitHub's
GraphQL API (a query per 100 pull requests, review decisions included), and
without one, its REST API (a request per 100 pull requests, and one per pull
request for its reviews, unless --no-reviews), which allows 60 requests an
hour.  Requests are serial; on a 403 or 429 it waits as GitHub says
(Retry-After, or until X-RateLimit-Reset when X-RateLimit-Remaining is 0,
else at least a minute, doubling), for at most --max-wait seconds (600 by
default) in all, and then stops, writing what it has with the repositories
it didn't finish marked incomplete.
"""

import argparse
import datetime
import json
import os
import re
import sys
import time
import tomllib
import urllib.error
import urllib.request

API = "https://api.github.com"
USER_AGENT = "searchfox-spec-prs/1.0 (+https://github.com/mozsearch/mozsearch)"

QUERY = """
query($owner: String!, $name: String!, $after: String) {
  repository(owner: $owner, name: $name) {
    pullRequests(states: OPEN, first: 100, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes {
        number title url isDraft authorAssociation reviewDecision
        headRefOid baseRefName updatedAt
        author { login }
        labels(first: 20) { nodes { name } }
      }
    }
  }
}
"""


def github_repo(url):
    """OWNER/NAME of a GitHub repository's URL, if it's one."""
    match = re.match(r"https://github\.com/([^/]+)/([^/]+?)(?:\.git)?/?$", url or "")
    return f"{match.group(1)}/{match.group(2)}" if match else None


def configured_repos(specs_toml, config_toml):
    """The GitHub repositories of a tree of specs' configured spec
    repositories (see the module's comment)."""
    with open(specs_toml, "rb") as f:
        urls = {spec["name"]: spec["url"] for spec in tomllib.load(f).get("spec", [])
                if spec.get("url")}
    with open(config_toml, "rb") as f:
        repos = tomllib.load(f).get("repos", [])
    return [github_repo(urls[repo]) for repo in repos if github_repo(urls.get(repo))]


class RateLimited(Exception):
    pass


class Client:
    def __init__(self, token, max_wait):
        self.token = token
        self.waited = 0
        self.max_wait = max_wait
        self.requests = 0

    def request(self, url, body=None):
        backoff = 60
        while True:
            headers = {"User-Agent": USER_AGENT, "Accept": "application/vnd.github+json"}
            if self.token:
                headers["Authorization"] = f"Bearer {self.token}"
            data = None
            if body is not None:
                data = json.dumps(body).encode()
                headers["Content-Type"] = "application/json"
            request = urllib.request.Request(url, data=data, headers=headers)
            self.requests += 1
            try:
                with urllib.request.urlopen(request, timeout=120) as response:
                    return json.loads(response.read())
            except urllib.error.HTTPError as e:
                if e.code not in (403, 429):
                    raise
                retry_after = e.headers.get("Retry-After", "")
                if retry_after.isdigit():
                    wait = int(retry_after)
                elif e.headers.get("X-RateLimit-Remaining") == "0":
                    reset = int(e.headers.get("X-RateLimit-Reset", "0") or 0)
                    wait = max(reset - int(time.time()), 1)
                else:
                    wait = backoff
                    backoff *= 2
                if self.waited + wait > self.max_wait:
                    raise RateLimited(f"rate limited ({e.code}); GitHub asked to wait {wait} s")
                print(f"Rate limited ({e.code}); waiting {wait} s", file=sys.stderr, flush=True)
                time.sleep(wait)
                self.waited += wait


def graphql_prs(client, owner, name):
    prs, after = [], None
    while True:
        result = client.request(f"{API}/graphql", {
            "query": QUERY, "variables": {"owner": owner, "name": name, "after": after}})
        if result.get("errors"):
            raise Exception(f"{owner}/{name}: {result['errors']}")
        connection = result["data"]["repository"]["pullRequests"]
        for node in connection["nodes"]:
            prs.append({
                "number": node["number"],
                "title": node["title"],
                "url": node["url"],
                "author": (node.get("author") or {}).get("login"),
                "association": node["authorAssociation"],
                "draft": node["isDraft"],
                "labels": [label["name"] for label in node["labels"]["nodes"]],
                "review": node["reviewDecision"],
                "head": node["headRefOid"],
                "base_ref": node["baseRefName"],
                "updated": node["updatedAt"],
            })
        if not connection["pageInfo"]["hasNextPage"]:
            return prs
        after = connection["pageInfo"]["endCursor"]


def review_decision(reviews):
    """The review decision of a pull request's reviews, as GraphQL's
    reviewDecision approximately says it: CHANGES_REQUESTED if a reviewer's
    latest decisive review requested changes, else APPROVED if one approved,
    else None (whether reviews are required isn't known)."""
    latest = {}
    for review in reviews:
        state = review.get("state")
        if state in ("APPROVED", "CHANGES_REQUESTED", "DISMISSED"):
            latest[(review.get("user") or {}).get("login")] = state
    states = set(latest.values())
    if "CHANGES_REQUESTED" in states:
        return "CHANGES_REQUESTED"
    if "APPROVED" in states:
        return "APPROVED"
    return None


def rest_prs(client, owner, name, reviews):
    prs, page = [], 1
    while True:
        batch = client.request(f"{API}/repos/{owner}/{name}/pulls?state=open&per_page=100&page={page}")
        for pr in batch:
            prs.append({
                "number": pr["number"],
                "title": pr["title"],
                "url": pr["html_url"],
                "author": (pr.get("user") or {}).get("login"),
                "association": pr.get("author_association"),
                "draft": pr.get("draft", False),
                "labels": [label["name"] for label in pr.get("labels", [])],
                "review": None,
                "head": pr["head"]["sha"],
                "base_ref": pr["base"]["ref"],
                "updated": pr["updated_at"],
            })
        if len(batch) < 100:
            break
        page += 1
    if reviews:
        rest_reviews(client, owner, name, prs)
    return prs


def rest_reviews(client, owner, name, prs):
    """Set pull requests' review decisions from their reviews (a request
    each)."""
    for pr in prs:
        found = client.request(f"{API}/repos/{owner}/{name}/pulls/{pr['number']}/reviews?per_page=100")
        pr["review"] = review_decision(found)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--no-reviews", action="store_true")
    parser.add_argument("--max-wait", type=int, default=600)
    parser.add_argument("--specs")
    parser.add_argument("--config")
    parser.add_argument("repos", nargs="*")
    args = parser.parse_args()
    if bool(args.specs) != bool(args.config):
        parser.error("--specs and --config go together")
    if args.specs:
        args.repos += configured_repos(args.specs, args.config)
    if not args.repos:
        parser.error("no repositories")
    client = Client(os.environ.get("GITHUB_TOKEN"), args.max_wait)
    snapshot = {
        "taken": datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "api": "graphql" if client.token else "rest",
        "repos": {},
    }
    limited = None
    for repo in args.repos:
        owner, name = repo.split("/", 1)
        if limited:
            snapshot["repos"][repo] = {"incomplete": True, "prs": []}
            continue
        try:
            if client.token:
                prs = graphql_prs(client, owner, name)
            else:
                prs = rest_prs(client, owner, name, not args.no_reviews)
            snapshot["repos"][repo] = {"incomplete": False, "prs": prs}
            print(f"{repo}: {len(prs)} open pull requests", file=sys.stderr, flush=True)
        except RateLimited as e:
            limited = str(e)
            snapshot["repos"][repo] = {"incomplete": True, "prs": []}
            print(f"{repo}: {e}; stopping", file=sys.stderr, flush=True)
    snapshot["requests"] = client.requests
    if limited:
        snapshot["limited"] = limited
    tmp = args.out + ".tmp"
    with open(tmp, "w") as f:
        json.dump(snapshot, f, indent=1, sort_keys=True)
        f.write("\n")
    os.replace(tmp, args.out)


if __name__ == "__main__":
    main()
