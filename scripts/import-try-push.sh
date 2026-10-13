#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Import a try push (or any head based on the tree's branch) whose commits are
# already in the tree's source repo into the tree's token-centric history and
# commit index, as the web server's try imports do (see tools/src/try_import.rs;
# the tree's config repo hook fetches the push, ex: from hg's try repository,
# and then runs this):
# - The source repo's `try` branch (see `source_mapping::TRY_BRANCH`) is set to
#   the push's head, and build-history.py processes it into the history repos'
#   `try` branch, whose notes accumulate every imported push, reading through to
#   the notes of the tree's branch so that only the push's own revisions are
#   processed (and those of its base which the history doesn't have yet, ex:
#   when it's based on a newer revision than the tree's, up to MAX_REVISIONS).
# - The commit index's `try` branch accumulates the pushes' commits, reading
#   through to the branch's index (see tools/src/commit_index.rs).
# - The push's own commits, oldest first, are written to
#   HISTORY_ROOT/try-pushes/NAME, for its page: the hook's PUSH_REVS (ex: from
#   the try repository's pushlog), or else those the branch doesn't have (which
#   includes its base's newer revisions).
# - Last, `refs/try/NAME` is set to the head in the source repo, so that its
#   existence means the push is imported (ex: NAME is its hg revision).
#
# Usage: import-try-push.sh SOURCE_GIT HISTORY_ROOT BRANCH NAME REV MAX_REVISIONS [SYNTAX_ARGS...]
# - BRANCH is the tree's `git_branch` (or HEAD, for the history repos' HEAD).
# - SYNTAX_ARGS are passed to build-syntax-token-tree, as for the tree's history
#   (ex: its history configuration directory), so the derived commits are the
#   same as they would be on the branch.

if [ $# -lt 6 ]; then
    echo "Usage: $0 SOURCE_GIT HISTORY_ROOT BRANCH NAME REV MAX_REVISIONS [SYNTAX_ARGS...]"
    exit 1
fi

SOURCE_GIT=$1
HISTORY_ROOT=$2
BRANCH=$3
NAME=$4
REV=$5
MAX_REVISIONS=$6
shift 6

TRY_BRANCH=try

HEAD_REV=$(git -C "$SOURCE_GIT" rev-parse --verify "$REV^{commit}")
if [[ "$BRANCH" == "HEAD" ]]; then
    SOURCE_REF=HEAD
    # (The history repos' branch, as the tools name the notes of HEAD's.)
    NOTES_BRANCH=$(git -C "$HISTORY_ROOT/timeline" symbolic-ref --short HEAD)
else
    SOURCE_REF="refs/heads/$BRANCH"
    NOTES_BRANCH=$BRANCH
fi

# The revisions the push has which the branch doesn't, which includes those of
# its base which are newer than the branch's head (the history has processed
# the branch up to its head, at the tree's last indexing).
NEW_REVISIONS=$(git -C "$SOURCE_GIT" rev-list --count "$HEAD_REV" --not "$SOURCE_REF")
echo "The push has $NEW_REVISIONS revisions which $BRANCH doesn't."
if (( NEW_REVISIONS > MAX_REVISIONS )); then
    echo "ERROR: more than $MAX_REVISIONS revisions to process; the push's base may be too far ahead of the tree's revision."
    exit 1
fi

git -C "$SOURCE_GIT" update-ref "refs/heads/$TRY_BRANCH" "$HEAD_REV"

BLAME_REF="refs/heads/$TRY_BRANCH" \
    READ_NOTES_REFS="refs/notes/mozsearch-source-mapping-$NOTES_BRANCH" \
    "$MOZSEARCH_PATH/scripts/build-history.py" "$SOURCE_GIT" "$HISTORY_ROOT" "$@"

build-commit-index "$SOURCE_GIT" "$HISTORY_ROOT/commit-index" "refs/heads/$TRY_BRANCH" \
    --accumulate --read "$BRANCH"

mkdir -p "$HISTORY_ROOT/try-pushes"
if [[ -n "${PUSH_REVS:-}" ]]; then
    tr ' ' '\n' <<< "$PUSH_REVS" | grep . > "$HISTORY_ROOT/try-pushes/$NAME"
else
    git -C "$SOURCE_GIT" rev-list --reverse "$HEAD_REV" --not "$SOURCE_REF" > "$HISTORY_ROOT/try-pushes/$NAME"
fi

git -C "$SOURCE_GIT" update-ref "refs/try/$NAME" "$HEAD_REV"
echo "Imported $NAME as $HEAD_REV."
