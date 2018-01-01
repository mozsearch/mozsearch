#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 3 ]
then
    echo "Usage: output.sh config_repo config-file.json tree_name"
    exit 1
fi

CONFIG_REPO=$(realpath $1)
CONFIG_FILE=$(realpath $2)
TREE_NAME=$3

cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files | \
    parallel --files --halt 2 -X --eta \
	     $MOZSEARCH_PATH/tools/target/release/output-file $CONFIG_FILE $TREE_NAME

HG_ROOT=$(python $MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/hg_root)
cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files > /tmp/dirs
js $MOZSEARCH_PATH/scripts/output-dir.js $FILES_ROOT $INDEX_ROOT "$HG_ROOT" $MOZSEARCH_PATH $OBJDIR $TREE_NAME /tmp/dirs

js $MOZSEARCH_PATH/scripts/output-template.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
js $MOZSEARCH_PATH/scripts/output-help.js $CONFIG_REPO/help.html $INDEX_ROOT $MOZSEARCH_PATH
