#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

mkdir /tmp/dummy
cd /tmp/dummy
ln -s $TREE_ROOT mozilla-central

$CODESEARCH $MOZSEARCH_ROOT/livegrep-index.json \
    -dump_index $INDEX_ROOT/livegrep.idx \
    -max_matches 1000 </dev/null

cd -
rm -rf /tmp/dummy
