#!/bin/bash

if [ $# -ne 2 ]
then
    echo "Usage: rust-analyze.sh config-file.json tree_name"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)
. $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

ANALYSIS_DIR=$(find $OBJDIR -type d -name save-analysis)

pushd $FILES_ROOT
$MOZSEARCH_PATH/tools/target/release/rust-indexer \
  $INDEX_ROOT \
  $ANALYSIS_DIR \
  $INDEX_ROOT/analysis
popd
