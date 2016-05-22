#!/bin/bash

if [ $# -ne 2 ]
then
    echo "Usage: output.sh config-file.json tree_name [file_filter]"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_FILE=$1
TREE_NAME=$2

FILTER=$3
if [ "x${FILTER}" = "x" ]
then
  FILTER=".*"
fi

cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files | grep "$FILTER" | \
    parallel --files --halt 2 -X --eta \
	     $MOZSEARCH_ROOT/tools/target/release/output-file $CONFIG_FILE $TREE_NAME

cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files | grep "$FILTER" > /tmp/dirs
$JS $MOZSEARCH_ROOT/output-dir.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT $OBJDIR /tmp/dirs

$JS $MOZSEARCH_ROOT/output-template.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT
$JS $MOZSEARCH_ROOT/output-help.js $TREE_ROOT $INDEX_ROOT $MOZSEARCH_ROOT
