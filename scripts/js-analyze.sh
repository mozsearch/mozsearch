#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

FILTER=$1
if [ "x${FILTER}" = "x" ]
then
  FILTER=".*"
fi

cat $INDEX_ROOT/js-files | grep "$FILTER" | \
    parallel --halt 2 js -f $MOZSEARCH_PATH/setversion.js \
    -f $MOZSEARCH_PATH/js-analyze.js -- {#} \
    $MOZSEARCH_PATH $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
