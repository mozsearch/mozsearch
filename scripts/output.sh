#!/bin/bash

if [ $# -ne 2 -a $# -ne 3 ]
then
    echo "Usage: output.sh config-file.json tree_name [file_filter]"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)
. $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

FILTER=$3
if [ "x${FILTER}" = "x" ]
then
    FILTER=".*"
fi

cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files | grep "$FILTER" | \
    parallel --files --halt 2 -X --eta \
	     $MOZSEARCH_PATH/tools/target/release/output-file $CONFIG_FILE $TREE_NAME

cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files > /tmp/dirs
js $MOZSEARCH_PATH/output-dir.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH $OBJDIR $TREE_NAME /tmp/dirs

js $MOZSEARCH_PATH/output-template.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
js $MOZSEARCH_PATH/output-help.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH
