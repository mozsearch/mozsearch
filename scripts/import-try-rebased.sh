#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Import a try push's stack rebased onto the tree's indexed revision
# (refs/try-rebased/NAME; see rebase-try-push.sh), whose head's pages have the
# push's semantic analysis (see tools/src/try_delta.rs), into the tree's
# token-centric history and commit index, as import-try-push.sh imports the
# push's own commits, so that those pages have the push's blame and stack strip
# too: its rebased commits are the push's commits, with their authors, dates
# and messages, on top of the indexed revision.  (A push on top of the indexed
# revision is its own rebased stack, which the history has already.)  The
# tree's import-try hook runs this after the push's semantic analysis.
#
# Usage: import-try-rebased.sh SOURCE_GIT HISTORY_ROOT BRANCH NAME MAX_REVISIONS [SYNTAX_ARGS...]
# (As import-try-push.sh's.)

if [ $# -lt 5 ]; then
    echo "Usage: $0 SOURCE_GIT HISTORY_ROOT BRANCH NAME MAX_REVISIONS [SYNTAX_ARGS...]"
    exit 1
fi

SOURCE_GIT=$1
HISTORY_ROOT=$2
BRANCH=$3
NAME=$4
MAX_REVISIONS=$5
shift 5

TRY_BRANCH=try
REBASED=$(git -C "$SOURCE_GIT" rev-parse --verify "refs/try-rebased/$NAME^{commit}")
if [[ "$BRANCH" == "HEAD" ]]; then
    SOURCE_REF=HEAD
    NOTES_BRANCH=$(git -C "$HISTORY_ROOT/timeline" symbolic-ref --short HEAD)
else
    SOURCE_REF="refs/heads/$BRANCH"
    NOTES_BRANCH=$BRANCH
fi

NEW_REVISIONS=$(git -C "$SOURCE_GIT" rev-list --count "$REBASED" --not "$SOURCE_REF")
echo "The push's rebased stack has $NEW_REVISIONS revisions which $BRANCH doesn't."
if (( NEW_REVISIONS > MAX_REVISIONS )); then
    echo "ERROR: more than $MAX_REVISIONS revisions to process."
    exit 1
fi

git -C "$SOURCE_GIT" update-ref "refs/heads/$TRY_BRANCH" "$REBASED"
BLAME_REF="refs/heads/$TRY_BRANCH" \
    READ_NOTES_REFS="refs/notes/mozsearch-source-mapping-$NOTES_BRANCH" \
    "$MOZSEARCH_PATH/scripts/build-history.py" "$SOURCE_GIT" "$HISTORY_ROOT" "$@"
build-commit-index "$SOURCE_GIT" "$HISTORY_ROOT/commit-index" "refs/heads/$TRY_BRANCH" \
    --accumulate --read "$BRANCH"
echo "Imported $NAME's rebased stack as $REBASED."
