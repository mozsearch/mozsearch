#!/bin/bash

if [ $# -ne 2 ]
then
    echo "Usage: ipdl-analyze.sh config-file.json tree_name"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

pushd $FILES_ROOT
cat $INDEX_ROOT/ipdl-files | \
    xargs $MOZSEARCH_PATH/tools/target/release/ipdl-analyze $(cat $INDEX_ROOT/ipdl-includes) \
          -d $INDEX_ROOT/analysis/__GENERATED__/ipc/ipdl/_ipdlheaders \
          -b $FILES_ROOT \
          -a $INDEX_ROOT/analysis
popd
