#!/bin/bash

if [ $# -ne 2 ]
then
    echo "Usage: js-analyze.sh config-file.json tree_name"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

cat $INDEX_ROOT/js-files | \
    parallel --halt 2 js -f $MOZSEARCH_PATH/js-analyze.js -- {#} \
    $MOZSEARCH_PATH $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
