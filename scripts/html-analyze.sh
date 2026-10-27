#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: html-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# Required by std::wcsrtombs, used in os.file.redirect.
export LC_CTYPE=C.UTF-8

# Each job (on each core, as parallel's default) takes 250 files: with
# `--pipe`'s default 1 MB blocks, firefox-disco's few MB of paths made only a
# handful of jobs (js-analyze ran at ~2.7 cores of 32 for a minute).
ANALYZE_FILES_PER_JOB=250

# Add line number for the file list with `nl`, which is used as a global
# fileIndex and used for local variable symbols.
#
# See the comment in js-analyze.sh for more details.
cat $INDEX_ROOT/html-files | nl -w1 -s " " | \
    parallel --pipe -N $ANALYZE_FILES_PER_JOB --halt 2 \
    js -f $MOZSEARCH_PATH/scripts/js-analyze.js -- \
    $MOZSEARCH_PATH $FILES_ROOT $INDEX_ROOT/analysis $MOZSEARCH_WASM_DIR
echo $?
