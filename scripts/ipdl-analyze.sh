#!/bin/bash

if [ $# -ne 2 -a $# -ne 3 ]
then
    echo "Usage: ipdl-analyze.sh config-file.json tree_name [file_filter]"
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

pushd $FILES_ROOT
cat $INDEX_ROOT/ipdl-files | grep "$FILTER" | \
    xargs $MOZSEARCH_PATH/tools/target/debug/ipdl-analyze $(cat $INDEX_ROOT/ipdl-includes) \
          -d $INDEX_ROOT/analysis/__GENERATED__/ipc/ipdl/_ipdlheaders \
          -b $FILES_ROOT \
          -a $INDEX_ROOT/analysis
popd
