#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# A try push's Java and Kotlin analysis, as the tree's (see
# process-java-artifacts.sh in the config: the per-file SCIP its Android
# build's scip-javac and scip-kotlin-analysis write, which `scip-java
# aggregate` makes one index of; see scip-indexer), for its semantic delta
# (see tools/src/try_delta.rs): of the files in FILES (ex: those it changes,
# and those whose SCIP its changes change), from SHARDS, its build's per-file
# SCIP of all of the files (`META-INF/scip/{path}.scip`, as a Taskcluster
# delta of the push has it), into OUT_DIR/{path} (ex: its Android platform's
# analysis, before it's merged, as the tree's Java and Kotlin analysis is its
# own platform's), added to the analysis there of the file, if any.  (All of
# the files' SCIP, rather than only theirs, as their uses of the others'
# symbols have those symbols' types from their definitions: the files'
# records are the tree's indexing's.  On firefox's 6,912 files, 22 s to
# aggregate and 14 s to index.)  The same scip-java as the tree's indexing's;
# the tree's older semanticdb indexes aren't supported.
#
# Usage: try-analyze-java.sh CONFIG_FILE TREE_NAME FILES_ROOT SHARDS FILES OUT_DIR

if [ $# -ne 6 ]; then
    echo "Usage: try-analyze-java.sh CONFIG_FILE TREE_NAME FILES_ROOT SHARDS FILES OUT_DIR" 1>&2
    exit 1
fi
CONFIG_FILE=$(realpath "$1")
TREE_NAME=$2
FILES_ROOT=$(realpath "$3")
SHARDS=$(realpath "$4")
FILES=$(realpath "$5")
OUT_DIR=$(realpath "$6")

if [ ! -d "$SHARDS/META-INF/scip" ]; then
    echo "No per-file SCIP in $SHARDS (META-INF/scip), so no Java or Kotlin analysis."
    exit 0
fi

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
scip-java aggregate --no-emit-inverse-relationships --output="$WORK/java.scip" "$SHARDS"
scip-indexer "$CONFIG_FILE" "$TREE_NAME" --subtree-root "." --analysis-root "$WORK/analysis" \
    --files-root "$FILES_ROOT" --no-leftovers "$WORK/java.scip"
N=0
while IFS= read -r FILE; do
    if [ -f "$WORK/analysis/$FILE" ]; then
        mkdir -p "$(dirname "$OUT_DIR/$FILE")"
        cat "$WORK/analysis/$FILE" >> "$OUT_DIR/$FILE"
        N=$((N + 1))
    fi
done < "$FILES"
echo "Analyzed $N of the push's $(wc -l < "$FILES") Java and Kotlin files."
