#!/usr/bin/env bash

set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Rebase a try push's stack onto the tree's indexed revision (its git repo's
# HEAD), for the push's semantic analysis (see tools/src/try_delta.rs), as
# refs/try-rebased/NAME, and print its commit: the push's head itself if the
# push is on top of the indexed revision, and otherwise its stack's commits
# (since its history diverged from the indexed revision's) applied to the
# indexed revision one at a time, leaving out those which have landed since
# then (as `git rebase` would): commits whose changes are a landed commit's
# (the same patch id), or which are the same Phabricator revision as one
# (their messages' "Differential Revision: <URL>" lines), even if it changed
# before it landed.  Which commits landed, and as what, it writes to stderr ("Landed:
# <commit> as <landed commit> (<how>)"), which the push's import log has (see
# tools/src/try_import.rs).  It fails if a commit's changes conflict
# ("Conflicting commit: <commit> <summary>", and git's lines for the files),
# or if all of the stack's commits have landed ("All of the push's commits
# have landed."), but those selecting try's tasks (changing only
# try_task_config.json), so there's nothing to analyze.
#
# Usage: rebase-try-push.sh GIT_ROOT NAME HEAD_REV

if [ $# -ne 3 ]; then
    echo "Usage: rebase-try-push.sh GIT_ROOT NAME HEAD_REV" 1>&2
    exit 1
fi

GIT_ROOT=$1
NAME=$2
HEAD_REV=$3

git() {
    command git -C "$GIT_ROOT" "$@"
}

BASE=$(git rev-parse HEAD)
FORK=$(git merge-base "$BASE" "$HEAD_REV")
if [ "$FORK" = "$BASE" ]; then
    git update-ref "refs/try-rebased/$NAME" "$HEAD_REV"
    echo "$HEAD_REV"
    exit 0
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

# What landed since the fork of the files the stack changes: those commits'
# patch ids ("<patch id> <commit>"), and their Phabricator revisions
# ("<commit> <revision URL>").
# (By the commits' files, rather than paths as arguments, which a big stack
# has too many of.)
git diff --name-only "$FORK" "$HEAD_REV" > "$WORK/files"
git log --no-merges --format='commit %H' --name-only "$FORK..$BASE" |
    awk 'NR == FNR { files[$0] = 1; next }
         /^commit / { commit = $2; next }
         ($0 in files) && !(commit in seen) { seen[commit] = 1; print commit }' \
        "$WORK/files" - > "$WORK/landed"
: > "$WORK/patch-ids"
: > "$WORK/revisions"
if [ -s "$WORK/landed" ]; then
    git log -p --no-merges --no-walk --stdin --format='commit %H' < "$WORK/landed" |
        git patch-id --stable > "$WORK/patch-ids"
    # (Its "Differential Revision: <URL>" line; not a trailer to git, whose
    # trailers' keys have no spaces.)
    git log --no-merges --no-walk --stdin --format='commit %H%n%B' < "$WORK/landed" |
        awk '/^commit [0-9a-f]+$/ { commit = $2; next }
             /^Differential Revision: / { print commit, $3 }' > "$WORK/revisions"
fi

export GIT_INDEX_FILE=$WORK/index
git read-tree "$BASE"
APPLIED=0
for COMMIT in $(git rev-list --reverse --no-merges "$FORK..$HEAD_REV"); do
    PATCH_ID=$(git show "$COMMIT" | git patch-id --stable | cut -d' ' -f1)
    LANDED=$(awk -v id="$PATCH_ID" '$1 == id { print $2; exit }' "$WORK/patch-ids")
    HOW="the same changes"
    if [ -z "$LANDED" ]; then
        REVISION=$(git log -1 --format=%B "$COMMIT" |
            awk '/^Differential Revision: / { revision = $3 } END { print revision }')
        if [ -n "$REVISION" ]; then
            LANDED=$(awk -v rev="$REVISION" '$2 == rev { print $1; exit }' "$WORK/revisions")
            HOW="its Differential Revision"
        fi
    fi
    if [ -n "$LANDED" ]; then
        echo "Landed: $COMMIT as $LANDED ($HOW)" 1>&2
        continue
    fi
    if ! git diff --binary --full-index "$COMMIT^" "$COMMIT" | git apply --cached --3way 1>&2; then
        echo "Conflicting commit: $COMMIT $(git log -1 --format=%s "$COMMIT")" 1>&2
        exit 1
    fi
    if [ "$(git diff-tree --no-commit-id --name-only -r "$COMMIT")" != "try_task_config.json" ]; then
        APPLIED=$((APPLIED + 1))
    fi
done
if [ $APPLIED -eq 0 ]; then
    echo "All of the push's commits have landed." 1>&2
    exit 1
fi

TREE=$(git write-tree)
unset GIT_INDEX_FILE
export GIT_AUTHOR_NAME="${GIT_AUTHOR_NAME:-Searchfox}"
export GIT_AUTHOR_EMAIL="${GIT_AUTHOR_EMAIL:-searchfox@localhost}"
export GIT_COMMITTER_NAME="${GIT_COMMITTER_NAME:-$GIT_AUTHOR_NAME}"
export GIT_COMMITTER_EMAIL="${GIT_COMMITTER_EMAIL:-$GIT_AUTHOR_EMAIL}"
REBASED=$(git commit-tree "$TREE" -p "$BASE" -m "Try push $NAME, rebased onto $BASE")
git update-ref "refs/try-rebased/$NAME" "$REBASED"
echo "$REBASED"
