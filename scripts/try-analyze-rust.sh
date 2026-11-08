#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's Rust analysis, as the tree's (rust-analyzer's SCIP index, see
# scip-indexer), for its semantic delta (see tools/src/try_delta.rs): of the
# files in FILES (try-rust-files' list of the files whose analysis the push
# can change: those it changes, and the tree's files using their symbols),
# from SCIP, rust-analyzer's index of them at its head (`rust-analyzer scip
# --only-files FILES`, with the tree's configuration, which the tree's
# indexing knows how to run; ex: in firefox's, its build's sandbox), into
# OUT_DIR/{path} (ex: its compilation's analysis, before it's merged, as the
# tree's Rust analysis is with its C++'s), added to the analysis there of the
# file, if any.  The structured records of the symbols the files don't define
# are left out (--no-leftovers), as the tree's analysis has them.
#
# Usage: try-analyze-rust.sh CONFIG_FILE TREE_NAME FILES_ROOT SCIP FILES OUT_DIR

if [ $# -ne 6 ]; then
    echo "Usage: try-analyze-rust.sh CONFIG_FILE TREE_NAME FILES_ROOT SCIP FILES OUT_DIR" 1>&2
    exit 1
fi
CONFIG_FILE=$(realpath "$1")
TREE_NAME=$2
FILES_ROOT=$(realpath "$3")
SCIP=$(realpath "$4")
FILES=$(realpath "$5")
OUT_DIR=$(realpath "$6")

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
scip-indexer "$CONFIG_FILE" "$TREE_NAME" --subtree-root "." --analysis-root "$WORK/analysis" \
    --files-root "$FILES_ROOT" --no-leftovers "$SCIP"
N=0
while IFS= read -r FILE; do
    if [ -f "$WORK/analysis/$FILE" ]; then
        mkdir -p "$(dirname "$OUT_DIR/$FILE")"
        cat "$WORK/analysis/$FILE" >> "$OUT_DIR/$FILE"
        N=$((N + 1))
    fi
done < "$FILES"
echo "Analyzed $N of the push's $(wc -l < "$FILES") Rust files."
