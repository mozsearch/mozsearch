# hg.mozilla.org's pushlogs, for hg-era landing dates

The history's dates are when revisions landed (see
`tools/src/hyperblame/landing.rs`).  Firefox's hg-era commits have their
authors' dates, which a patch kept when it landed weeks later or was relanded
after a backout, and the computed landing dates (the latest of a commit's and
its ancestors' commit dates) are only lower bounds, which an author's clock
running ahead carries forward to everything after it.  hg.mozilla.org's
pushlogs (version-control-tools' `hgext/pushlog`, its `json-pushes` command)
have when each changeset was pushed, which is when it landed.

Since Firefox moved to git (2025), hg.mozilla.org is a mirror, and its pushes
replicate git's landings, whose commit dates are when they landed, so the
pushlogs only matter until then, and were archived once:

1. `scrape_pushlog.py ARCHIVE` archives mozilla-central's, the integration
   repos' and the release repos' pushlogs as gzipped JSON chunks of 1,000
   pushes (587 requests).  It's resumable, and hg.mozilla.org's WAF answers
   429 when it rate-limits (about 10 minutes of cool-off), which it waits out
   before going on more slowly.
2. `derive_push_dates.py ARCHIVE GIT_HG OUT --until DATE` writes the push dates
   file: each git revision's first push, from GIT_HG's
   `GIT_REV<TAB>HG_REV` lines (ex: from the history's timeline commits'
   messages), leaving out pushes from the git migration on and pushes of
   imported history (see its comment).
   `--graph GIT_REPO` bounds each date by its children's pushes, since the
   pushlogs miss a few pushes.
3. `validate_push_dates.py OUT GIT_REPO` checks it against the commit graph:
   parents pushed after their children, and push dates against commit dates
   and computed landing dates, by year.  (`cutover.py` is how the git
   migration's cutover, 2025-06-01, was chosen.)

The archive of 2026-10-25 (24 repos, 148 MB) gave 724,772 of main's commits
push dates, with:
```
derive_push_dates.py ARCHIVE git-hg-main.tsv.gz firefox-hg-push-dates.tsv \
    --until 2025-06-01 --graph FIREFOX_GIT
```
where git-hg-main.tsv.gz is from the firefox history's timeline commits'
`git`/`hg` lines.

The push dates file is `s3://searchfox.repositories/firefox-hg-push-dates.tsv.gz`
(like the CVS revisions' map), which the firefox-disco config's
`update-history.sh` downloads for build-timeline-tree (`PUSH_DATES`) and
build-commit-index (`--push-dates`).
