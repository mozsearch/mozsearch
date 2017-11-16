#!/bin/bash

if [ $# -ne 3 -a $# -ne 4 ]
then
    echo "Usage: output.sh config_repo config-file.json tree_name [file_filter]"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_REPO=$(realpath $1)
CONFIG_FILE=$(realpath $2)
TREE_NAME=$3

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)
. $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

FILTER=$4
if [ "x${FILTER}" = "x" ]
then
    FILTER=".*"
fi

cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files | grep "$FILTER" | \
    parallel --files --halt 2 -X --eta \
	     $MOZSEARCH_PATH/tools/target/release/output-file $CONFIG_FILE $TREE_NAME

if [ "${FILTER}" = ".*" ]
then
    HG_ROOT=$(python $MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/hg_root)
    cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files > /tmp/dirs
    js $MOZSEARCH_PATH/output-dir.js $FILES_ROOT $INDEX_ROOT $HG_ROOT $MOZSEARCH_PATH $OBJDIR $TREE_NAME /tmp/dirs

    js $MOZSEARCH_PATH/output-template.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
    js $MOZSEARCH_PATH/output-help.js $CONFIG_REPO/help.html $INDEX_ROOT $MOZSEARCH_PATH
fi
