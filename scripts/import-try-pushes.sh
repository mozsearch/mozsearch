#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Import try pushes (or any heads based on the tree's branch) whose commits are
# already in the tree's source repo into the tree's token-centric history and
# commit index, as the web server's try imports do (see tools/src/try_import.rs;
# the tree's config repo hook fetches the push, ex: from hg's try repository,
# and then runs scripts/import-try-push.sh, a batch of one, or several spec
# pull requests' at once by scripts/import-try-push-batched.sh), all of their
# revisions in one history run:
# - The source repo's `try` branch (see `source_mapping::TRY_BRANCH`) is set to
#   the last push's head (and the others are the tools' EXTRA_HEADS), and
#   build-history.py processes them into the history repos' `try` branch, whose
#   notes accumulate every imported push, reading through to the notes of the
#   tree's branch so that only the pushes' own revisions are processed (and
#   those of their bases which the history doesn't have yet, ex: when one's
#   based on a newer revision than the tree's, up to MAX_REVISIONS each).
#   Processing them together saves each one's runs' starts and ends, and lets
#   the tools compute their revisions at once.
# - The commit index's `try` branch accumulates the pushes' commits, reading
#   through to the branch's index (see tools/src/commit_index.rs).
# - Each push's stack, its commits since it diverged from the trunk, oldest
#   first, is written to HISTORY_ROOT/try-pushes/NAME, for its page: its
#   PUSH_REVS (ex: the try repository's draft ancestors of the head, which
#   include the stack's commits earlier pushes had), or else the commits the
#   branch doesn't have (which include its base's newer revisions, if any).
#   Commits which only select try's tasks (only changing try_task_config.json,
#   ex: "Tasks automatically selected.") are listed as "-REV", which the page
#   leaves out.
# - Last, each push's `refs/try/NAME` is set to its head in the source repo, so
#   that its existence means the push is imported (ex: NAME is its hg
#   revision, or a spec's pull request's SPEC/NUMBER; see build-spec-pr).
#
# Usage: import-try-pushes.sh SOURCE_GIT HISTORY_ROOT BRANCH MAX_REVISIONS BATCH [SYNTAX_ARGS...]
# - BRANCH is the tree's `git_branch` (or HEAD, for the history repos' HEAD).
# - BATCH is a file of the pushes, a line each: its NAME, its head REV, and its
#   PUSH_REVS (if any), separated by spaces.
# - SYNTAX_ARGS are passed to build-syntax-token-tree, as for the tree's history
#   (ex: its history configuration directory), so the derived commits are the
#   same as they would be on the branch.

if [ $# -lt 5 ]; then
    echo "Usage: $0 SOURCE_GIT HISTORY_ROOT BRANCH MAX_REVISIONS BATCH [SYNTAX_ARGS...]"
    exit 1
fi

SOURCE_GIT=$1
HISTORY_ROOT=$2
BRANCH=$3
MAX_REVISIONS=$4
BATCH=$5
shift 5

TRY_BRANCH=try

if [[ "$BRANCH" == "HEAD" ]]; then
    SOURCE_REF=HEAD
    # (The history repos' branch, as the tools name the notes of HEAD's.)
    NOTES_BRANCH=$(git -C "$HISTORY_ROOT/timeline" symbolic-ref --short HEAD)
else
    SOURCE_REF="refs/heads/$BRANCH"
    NOTES_BRANCH=$BRANCH
fi

NAMES=()
HEADS=()
STACKS=()
while read -r NAME REV PUSH_REVS; do
    [ -n "$NAME" ] || continue
    HEAD_REV=$(git -C "$SOURCE_GIT" rev-parse --verify "$REV^{commit}")
    # The revisions the push has which the branch doesn't, which includes
    # those of its base which are newer than the branch's head (the history
    # has processed the branch up to its head, at the tree's last indexing).
    NEW_REVISIONS=$(git -C "$SOURCE_GIT" rev-list --count "$HEAD_REV" --not "$SOURCE_REF")
    echo "$NAME has $NEW_REVISIONS revisions which $BRANCH doesn't."
    if (( NEW_REVISIONS > MAX_REVISIONS )); then
        echo "ERROR: $NAME has more than $MAX_REVISIONS revisions to process; its base may be too far ahead of the tree's revision."
        exit 1
    fi
    NAMES+=("$NAME")
    HEADS+=("$HEAD_REV")
    STACKS+=("$PUSH_REVS")
done < "$BATCH"
if [ ${#NAMES[@]} -eq 0 ]; then
    echo "ERROR: no pushes to import"
    exit 1
fi
LAST=$(( ${#HEADS[@]} - 1 ))
echo "Importing ${#NAMES[@]} push(es): ${NAMES[*]}"

git -C "$SOURCE_GIT" update-ref "refs/heads/$TRY_BRANCH" "${HEADS[$LAST]}"

# (Its repacks run in the background, after it's done, rather than holding up
# the import; see build-history.py's HISTORY_DETACHED_REPACKS.  The other
# pushes' heads are the tools' EXTRA_HEADS.)
BLAME_REF="refs/heads/$TRY_BRANCH" \
    EXTRA_HEADS="${HEADS[*]:0:$LAST}" \
    READ_NOTES_REFS="refs/notes/mozsearch-source-mapping-$NOTES_BRANCH" \
    HISTORY_DETACHED_REPACKS=1 \
    "$MOZSEARCH_PATH/scripts/build-history.py" "$SOURCE_GIT" "$HISTORY_ROOT" "$@"

# (A branch at a time, the try branch at each push's head; the last's last.)
for i in "${!HEADS[@]}"; do
    git -C "$SOURCE_GIT" update-ref "refs/heads/$TRY_BRANCH" "${HEADS[$i]}"
    build-commit-index "$SOURCE_GIT" "$HISTORY_ROOT/commit-index" "refs/heads/$TRY_BRANCH" \
        --accumulate --read "$BRANCH"
done

for i in "${!NAMES[@]}"; do
    NAME=${NAMES[$i]}
    HEAD_REV=${HEADS[$i]}
    # (A spec's pull request's NAME is SPEC/NUMBER, in its spec's directory.)
    mkdir -p "$(dirname "$HISTORY_ROOT/try-pushes/$NAME")"
    if [[ -n "${STACKS[$i]}" ]]; then
        STACK=$(tr ' ' '\n' <<< "${STACKS[$i]}" | grep .)
    else
        STACK=$(git -C "$SOURCE_GIT" rev-list --reverse "$HEAD_REV" --not "$SOURCE_REF")
    fi
    for COMMIT in $STACK; do
        if [[ "$(git -C "$SOURCE_GIT" diff-tree --no-commit-id --name-only -r "$COMMIT")" == "try_task_config.json" ]]; then
            echo "-$COMMIT"
        else
            echo "$COMMIT"
        fi
    done > "$HISTORY_ROOT/try-pushes/$NAME"

    git -C "$SOURCE_GIT" update-ref "refs/try/$NAME" "$HEAD_REV"
    echo "Imported $NAME as $HEAD_REV."
done
