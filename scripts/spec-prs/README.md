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

The Action lives in a repository of its own. It publishes the snapshot as the
single commit of its `snapshot` branch, force-pushed each time, so the
repository doesn't grow. Its list of repositories is the tree's
configuration (the config repo's `{tree}/specs.toml` and
`{tree}/spec-prs.toml`), so there's only one list. With its token, it asks
the GraphQL API: one query per 100 pull requests, review decisions included.
On a 403 or 429, it waits as GitHub says, for at most `--max-wait` seconds,
then publishes what it has, with the repositories it didn't finish marked
incomplete.

`.github/workflows/snapshot.yml`:

```yaml
name: Snapshot spec pull requests
on:
  schedule:
    - cron: "23 * * * *"
  workflow_dispatch:
permissions:
  contents: write
concurrency:
  group: snapshot
env:
  # The config repo with the tree's specs.toml and spec-prs.toml.
  CONFIG_REPOSITORY: mozsearch/mozsearch-mozilla
  CONFIG_REF: master
  TREE: wubspecs
jobs:
  snapshot:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
        with:
          repository: mozsearch/mozsearch
          path: mozsearch
          sparse-checkout: scripts/spec-prs
      - uses: actions/checkout@v4
        with:
          repository: ${{ env.CONFIG_REPOSITORY }}
          ref: ${{ env.CONFIG_REF }}
          path: config
          sparse-checkout: ${{ env.TREE }}
      - name: Snapshot
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          python3 mozsearch/scripts/spec-prs/snapshot.py --out snapshot.json \
            --specs "config/$TREE/specs.toml" --config "config/$TREE/spec-prs.toml"
      - name: Publish
        run: |
          mkdir publish
          mv snapshot.json publish/
          cd publish
          git init -q -b snapshot
          git config user.name "github-actions[bot]"
          git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
          git add snapshot.json
          git commit -q -m "Snapshot of the spec repositories' open pull requests"
          git push -q --force \
            "https://x-access-token:${{ secrets.GITHUB_TOKEN }}@github.com/${{ github.repository }}" \
            snapshot
```

The snapshot is about 300 bytes per pull request: about 2 MB for every open
pull request of wubspecs' 391 repositories, and much less for a configured
set.
