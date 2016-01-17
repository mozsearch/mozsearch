#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

FILTER=$1
if [ "x${FILTER}" = "x" ]
then
  FILTER=".*"
fi

cat $INDEX_ROOT/js-files | grep "$FILTER" | \
    parallel $JS -f $MOZSEARCH_ROOT/setversion.js \
    -f $MOZSEARCH_ROOT/js-analyze.js -- {#} \
    $MOZSEARCH_ROOT $TREE_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
