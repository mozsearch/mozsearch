#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

ANALYSIS_FILES_PATH=$1

# The structured records' canGC and gcPath, from the hazard analysis'
# gcFunctions.txt and allFunctions.txt, if the tree has them; see
# tools/src/bin/annotate-gc.rs, which loads them once (2.9 GB for firefox) and
# annotates the files in parallel.
if [ -f $INDEX_ROOT/gcFunctions.txt -a -f $INDEX_ROOT/allFunctions.txt ]; then
    annotate-gc $INDEX_ROOT/analysis \
        $INDEX_ROOT/gcFunctions.txt $INDEX_ROOT/allFunctions.txt < $ANALYSIS_FILES_PATH
fi
