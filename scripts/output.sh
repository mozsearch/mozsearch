#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

FILTER=$1
if [ "x${FILTER}" = "x" ]
then
  FILTER=".*"
fi

cat $INDEX_ROOT/all-files | grep "$FILTER" | \
    parallel --halt 2 -X --eta \
    $JS $MOZSEARCH_ROOT/output-file.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT

cat $INDEX_ROOT/all-dirs | grep "$FILTER" | \
    parallel --halt 2 -X --eta \
    $JS $MOZSEARCH_ROOT/output-dir.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT

$JS $MOZSEARCH_ROOT/output-template.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT
$JS $MOZSEARCH_ROOT/output-help.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT
