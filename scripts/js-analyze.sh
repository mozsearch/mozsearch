#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ] && [ $# -ne 3 ]
then
    echo "Usage: js-analyze.sh config-file.json tree_name [files]"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2
# (Only these of the tree's js-files, if given, ex: those whose analyses
# aren't another tree's; see web-analysis-share.sh.)
ONLY_FILES=${3:-}

# Required by std::wcsrtombs, used in os.file.redirect.
export LC_CTYPE=C.UTF-8

# Each job (on each core, as parallel's default) takes 250 files: with
# `--pipe`'s default 1 MB blocks, firefox-disco's few MB of paths made only a
# handful of jobs (js-analyze ran at ~2.7 cores of 32 for a minute).
ANALYZE_FILES_PER_JOB=250

# (The pages whose inline scripts and event handlers their JS environments'
# analysis has, in js-environments-pages/, get only their markup's analysis
# here; see js-environments-analyze.sh.  The custom properties of the tree's
# tests' styles, under its wpt_root, are each document's own; "-" if it has
# none.)
TESTS_ROOT=$(jq -r --arg tree "$TREE_NAME" '.trees[$tree].wpt_root // "-"' "$CONFIG_FILE")
#
# Add line number for the file list with `nl`, which is used as a global
# fileIndex, and used for local variable symbols.
#
# NOTE: The index conflicts with html-analyze.sh.
#       Currently this is not a problem because the index is used only for
#       local variables with no_crossref symbols.
numbered_files() {
    if [ -n "$ONLY_FILES" ]; then
        nl -w1 -s " " < $INDEX_ROOT/js-files |
            awk 'NR == FNR { only[$0]; next } { path = $0; sub(/^[0-9]+ /, "", path) } path in only' "$ONLY_FILES" -
    else
        nl -w1 -s " " < $INDEX_ROOT/js-files
    fi
}
numbered_files | \
    parallel --pipe -N $ANALYZE_FILES_PER_JOB --halt 2 \
    js -f $MOZSEARCH_PATH/scripts/js-analyze.js -- \
    $MOZSEARCH_PATH $FILES_ROOT $INDEX_ROOT/analysis $MOZSEARCH_WASM_DIR \
    $INDEX_ROOT/js-environments-pages "$TESTS_ROOT"
echo $?
