#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

ANALYSIS_FILES_PATH=$1

DIAGS_DIR=$INDEX_ROOT/diags/replace-aliases
mkdir -p $DIAGS_DIR
# clean up the directory since in the VM this can persist.
rm -f $DIAGS_DIR/*

JOBLOG_PATH=${DIAGS_DIR}/replace-aliases.joblog
TMPDIR_PATH=${DIAGS_DIR}

parallel --jobs 8 --pipepart -a $ANALYSIS_FILES_PATH --files --joblog $JOBLOG_PATH --tmpdir $TMPDIR_PATH \
    --block -1 --halt 2 \
    "$MOZSEARCH_PATH/scripts/replace-aliases.py $INDEX_ROOT/analysis $INDEX_ROOT/aliases/url-map.json"
