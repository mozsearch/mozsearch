#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

ANALYSIS_FILES_PATH=$1

if [ -f $INDEX_ROOT/gcFunctions.txt -a -f $INDEX_ROOT/allFunctions.txt ]; then
    cat $ANALYSIS_FILES_PATH | \
        parallel --jobs 16 --pipe --halt 2 \
        $MOZSEARCH_PATH/scripts/annotate-gc.py \
        $INDEX_ROOT/analysis \
        $INDEX_ROOT/gcFunctions.txt $INDEX_ROOT/allFunctions.txt
fi
