#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's TypeScript and JS analysis, for its semantic delta (see
# tools/src/try_delta.rs), as the tree's indexing makes its `typescript` scip
# subtree's (with scip-typescript-index.sh, and scip-indexer): of the files in
# CHANGED (a list of paths, ex: those the push's semantic analysis builds) at
# FILES_ROOT (its head's checkout) with TypeScript's or JS's extensions, by
# an index of only them (scip-typescript's --only-files, loading only the
# projects which include them), with PROJECTS (see scip-typescript-index.sh;
# ex: those at its head, listed as the tree's indexing lists them), into
# OUT_DIR/{path}, whose documents are a full index's (its uses of other
# files' symbols have their symbol information as the index's external
# symbols; see scip-typescript), without the structured records of the
# symbols they don't define (scip-indexer's --no-leftovers), which the tree's
# analysis has.  Its JS which the projects don't include is then analyzed as
# the tree's is (see try-analyze-js.sh, which leaves out the files OUT_DIR has).
# A tree without a `typescript` scip subtree has none of it.
#
# Usage: try-analyze-typescript.sh CONFIG_FILE TREE_NAME FILES_ROOT PROJECTS CHANGED OUT_DIR

if [ $# -ne 6 ]; then
    echo "Usage: try-analyze-typescript.sh CONFIG_FILE TREE_NAME FILES_ROOT PROJECTS CHANGED OUT_DIR" 1>&2
    exit 1
fi

CONFIG_FILE=$1
TREE_NAME=$2
FILES_ROOT=$3
PROJECTS=$4
CHANGED=$5
OUT_DIR=$6

SUBTREE=$(jq -c --arg tree "$TREE_NAME" '.trees[$tree].scip_subtrees.typescript // empty' "$CONFIG_FILE")
if [ -z "$SUBTREE" ]; then
    echo "The tree has no TypeScript analysis (a typescript scip subtree)."
    exit 0
fi
SUBTREE_ROOT=$(jq -r '.subtree_root' <<< "$SUBTREE")
ROOT=$FILES_ROOT${SUBTREE_ROOT:+/$SUBTREE_ROOT}

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
# (Its files' paths relative to the subtree's root; not those it removes.)
PREFIX=${SUBTREE_ROOT:+$SUBTREE_ROOT/}
{ grep -E '\.(js|jsm|mjs|cjs|jsx|ts|tsx|mts|cts)$' "$CHANGED" || true; } |
    while read -r FILE; do
        if [[ $FILE == "$PREFIX"* ]] && [ -f "$FILES_ROOT/$FILE" ]; then
            echo "${FILE#"$PREFIX"}"
        fi
    done > "$WORK/only-files"
echo "Analyzing the push's $(wc -l < "$WORK/only-files") TypeScript and JS files with scip-typescript."
if [ ! -s "$WORK/only-files" ]; then
    exit 0
fi

"$MOZSEARCH_PATH/scripts/scip-typescript-index.sh" "$ROOT" "$PROJECTS" \
    "$WORK/typescript.scip" "$WORK/only-files"
scip-indexer "$CONFIG_FILE" "$TREE_NAME" --subtree-name typescript \
    --subtree-root "$SUBTREE_ROOT" --analysis-root "$OUT_DIR" \
    --files-root "$FILES_ROOT" --no-leftovers "$WORK/typescript.scip"
