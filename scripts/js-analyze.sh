#!/bin/bash

if [ $# -ne 2 -a $# -ne 3 ]
then
    echo "Usage: js-analyze.sh config-file.json tree_name [file_filter]"
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

cat $INDEX_ROOT/js-files | grep "$FILTER" | \
    parallel --halt 2 js -f $MOZSEARCH_PATH/js-analyze.js -- {#} \
    $MOZSEARCH_PATH $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
