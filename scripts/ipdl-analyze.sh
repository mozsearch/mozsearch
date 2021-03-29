#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: ipdl-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

pushd $FILES_ROOT
cat $INDEX_ROOT/ipdl-files | \
    xargs $MOZSEARCH_PATH/tools/target/release/ipdl-analyze $(cat $INDEX_ROOT/ipdl-includes) \
          -d $INDEX_ROOT/analysis/__GENERATED__/ipc/ipdl/_ipdlheaders \
          -f $INDEX_ROOT/repo-files \
          -b $FILES_ROOT \
          -a $INDEX_ROOT/analysis
popd
