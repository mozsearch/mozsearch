#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

cd $(dirname $TREE_ROOT)
$CODESEARCH $MOZSEARCH_ROOT/livegrep-index.json \
    -dump_index $INDEX_ROOT/livegrep.idx \
    -max_matches 1000 </dev/null
