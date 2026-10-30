# Spec pull request summaries

A tree of specs (a forest of spec repositories, like wubspecs; see
tools/src/spec_forest.rs) can show summaries of its spec repositories' open
pull requests, at `/{tree}/spec-prs` and `/{tree}/spec-prs/{spec}` (see
tools/src/spec_prs.rs). They're filterable by review decision, by whether
their authors can review, by draft status, labels, specs and sections. Each
links to its page as a try push, `/{tree}/try/{spec}/{number}`, which imports
it (see tools/src/try_import.rs). The summaries cover only the pull requests'
current heads. They're not a history.

They come from two places:

- **Metadata.** Titles, authors and their associations with the repository,
  drafts, labels and review decisions come from GitHub's API. A scheduled
  GitHub Action asks for them, with its own `GITHUB_TOKEN`, and publishes a
  snapshot (snapshot.py). Searchfox never handles a token: a sync only
  downloads the snapshot, which is public data.
- **Changes.** What each pull request changes relative to its spec's trunk
  in the forest (files, hunks, the sections they're in) comes from git. Its
  head is fetched (`refs/pull/N/head`, through git, outside the API's rate
  limits) and diffed from its merge base with the trunk (sync.py).

## Syncing

The web server runs the tree's `sync-spec-prs` hook (in its config repo
directory) when asked: a POST to `/{tree}/spec-prs/sync`, or the sync buttons
on the pages. It runs one sync at a time, and at most one every 2 minutes for
the same repositories, with the hook's output in
`HISTORY_ROOT/spec-prs/sync.log`. The hook gets the environment variables
MOZSEARCH_PATH, CONFIG_REPO, TREE_NAME, GIT_ROOT, HISTORY_ROOT and
INDEX_ROOT. With `?spec=SPEC`, it gets one spec repository's directory as its
argument. It runs sync.py:

    sync.py --specs SPECS_TOML --config SPEC_PRS_TOML --forest "$GIT_ROOT" \
            --out "$HISTORY_ROOT/spec-prs" ${1:+--spec "$1"}

SPEC_PRS_TOML (ex: `{config}/{tree}/spec-prs.toml`) configures the sync:

    # The forest directories whose pull requests are summarized.
    repos = ["dom", "html", "ecma262", "csswg-drafts"]
    # The Action's snapshot (a URL, or a path relative to this file).
    snapshot = "https://raw.githubusercontent.com/OWNER/STATE_REPO/snapshot/snapshot.json"

A sync reads the snapshot, then fetches each configured repository's pull
requests' heads, one repository at a time. It writes
`HISTORY_ROOT/spec-prs/SPEC.json`, keeping the summary of any pull request
whose head and trunk haven't changed. A repository missing from the snapshot
(or every repository, if the snapshot can't be read) is skipped. With
`--spec`, it's asked of GitHub's REST API instead, unauthenticated (60
requests an hour): a request per 100 pull requests, and one per pull request
for its reviews, if it has at most 40.

## The GitHub Action

The Action is [mozsearch/spec-pr-state](https://github.com/mozsearch/spec-pr-state)'s
[snapshot.yml](https://github.com/mozsearch/spec-pr-state/blob/main/.github/workflows/snapshot.yml),
which runs hourly (or from its "Run workflow" button). It checks out this
directory's snapshot.py (from mozsearch's `hyperblame` branch) and the tree's
configuration (the config repo's `wubspecs/specs.toml` and
`wubspecs/spec-prs.toml`, from mozsearch-mozilla's `firefox-disco` branch),
so the list of repositories is only in the tree's configuration. It publishes
the snapshot as the single commit of its `snapshot` branch, force-pushed each
time, so the repository doesn't grow. With its token, it asks the GraphQL
API: one query per 100 pull requests, review decisions included. On a 403 or
429, it waits as GitHub says, for at most `--max-wait` seconds, then
publishes what it has, with the repositories it didn't finish marked
incomplete. Its first run (2026-10-26) took 90 queries for wubspecs' 85
repositories' 1,239 open pull requests.

The snapshot is about 440 bytes per pull request: 540 KB for wubspecs' 85
repositories' 1,239 open pull requests (2026-10-26).

## Merged pull requests

The blame popups link a forest's commit to its GitHub pull request and the
issues that closes, before its diff (see tools/src/pull_requests.rs and
`blame::commit_info_json`). Most spec repositories' commits don't say which
pull request they are. WHATWG's squash merges' subjects don't have "(#N)",
for example, though their messages often say what they fix ("Fixes #123.").
So the Action also snapshots the merged pull requests of all of the tree's
specs' GitHub repositories (merged.py): each one's number, its merge commit
(for a squash merge, the squashed commit), and the issues it closes (GitHub's
closing references, from the pull request's description or linked by hand).
They're published as `merged.json` beside `snapshot.json`.

    merged.py --specs SPECS_TOML --out merged.json --previous PREVIOUS_MERGED_JSON

With the previous snapshot (the last one published), it asks only for the
pull requests updated since each repository's newest: the 25 newest of 20
repositories at a time (5 points of the 1,000 an hour the Action's token
has), and then 100 at a time (a point), for those with more. So an hourly
run of wubspecs' 391 repositories takes about 20 queries (100 points). A
first run pages through every merged pull request: wubspecs' 52,281
(2026-10-29; whatwg/html's 3,570, w3c/csswg-drafts' 2,648) take 768 queries,
a point each, so about two hours' runs, since it stops after `--budget`
seconds, or with fewer than `--reserve` points left (200, for the next run's
open pull requests), and continues where it was the next hour (`cursor`).
The snapshot is then about 2.8 MB.

The snapshot is about 60 bytes per pull request:

    {"taken": "...", "repos": {"whatwg/html": {"specs": ["html"],
      "newest": "2026-10-29T...Z", "cursor": null,
      "prs": [[13006, "57535533d504...", ["#12951", "w3c/hr-time#169"]], ...]}}}

The tree's indexing downloads it into its history directory, as
`pull-requests.json` (the config repo's `wubspecs/setup`), and a commit's
pull request is the one whose merge commit is the commit's upstream commit
(its `Upstream:` trailer's). Failing that, it's the one its subject names, as
GitHub's merges' subjects do ("Fix it (#123)", "Merge pull request #123
from ..."). Its issues are the pull request's closing references and those
its message closes with GitHub's keywords ("Fixes #45", "closes
whatwg/dom#6", "Fixes https://github.com/whatwg/html/issues/7"). Without the
snapshot, only what the messages say is linked.
