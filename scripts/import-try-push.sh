#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Import a try push (or any head based on the tree's branch) whose commits are
# already in the tree's source repo into the tree's token-centric history and
# commit index, as the web server's try imports do (see tools/src/try_import.rs;
# the tree's config repo hook fetches the push, ex: from hg's try repository,
# and then runs this): a batch of one push for scripts/import-try-pushes.sh
# (see it for what an import does), named NAME (ex: its hg revision, or a
# spec's pull request's SPEC/NUMBER; see build-spec-pr), whose head is REV,
# with PUSH_REVS, if set, its stack (ex: the try repository's draft ancestors
# of the head).
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

BATCH=$(mktemp)
trap 'rm -f "$BATCH"' EXIT
echo "$NAME $REV ${PUSH_REVS:-}" > "$BATCH"
"$MOZSEARCH_PATH/scripts/import-try-pushes.sh" "$SOURCE_GIT" "$HISTORY_ROOT" "$BRANCH" \
    "$MAX_REVISIONS" "$BATCH" "$@"
