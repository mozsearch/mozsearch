#!/usr/bin/env bash

set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Rebase a try push's stack onto the tree's indexed revision (its git repo's
# HEAD), for the push's semantic analysis (see tools/src/try_delta.rs), as
# refs/try-rebased/NAME, and print its commit: the push's head itself if the
# push is on top of the indexed revision, and otherwise a commit of the
# stack's changes (since its history diverged from the indexed revision's)
# applied to the indexed revision, which fails if they conflict.
#
# Usage: rebase-try-push.sh GIT_ROOT NAME HEAD_REV

if [ $# -ne 3 ]; then
    echo "Usage: rebase-try-push.sh GIT_ROOT NAME HEAD_REV" 1>&2
    exit 1
fi

GIT_ROOT=$1
NAME=$2
HEAD_REV=$3

BASE=$(git -C "$GIT_ROOT" rev-parse HEAD)
FORK=$(git -C "$GIT_ROOT" merge-base "$BASE" "$HEAD_REV")
if [ "$FORK" = "$BASE" ]; then
    REBASED=$HEAD_REV
else
    export GIT_INDEX_FILE=$(mktemp)
    git -C "$GIT_ROOT" read-tree "$BASE"
    git -C "$GIT_ROOT" diff --binary --full-index "$FORK" "$HEAD_REV" |
        git -C "$GIT_ROOT" apply --cached --3way
    TREE=$(git -C "$GIT_ROOT" write-tree)
    rm -f "$GIT_INDEX_FILE"
    unset GIT_INDEX_FILE
    export GIT_AUTHOR_NAME="${GIT_AUTHOR_NAME:-Searchfox}"
    export GIT_AUTHOR_EMAIL="${GIT_AUTHOR_EMAIL:-searchfox@localhost}"
    export GIT_COMMITTER_NAME="${GIT_COMMITTER_NAME:-$GIT_AUTHOR_NAME}"
    export GIT_COMMITTER_EMAIL="${GIT_COMMITTER_EMAIL:-$GIT_AUTHOR_EMAIL}"
    REBASED=$(git -C "$GIT_ROOT" commit-tree "$TREE" -p "$BASE" \
        -m "Try push $NAME, rebased onto $BASE")
fi
git -C "$GIT_ROOT" update-ref "refs/try-rebased/$NAME" "$REBASED"
echo "$REBASED"
