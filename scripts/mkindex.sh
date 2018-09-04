#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file> <tree-name>"
    exit 1
fi

CONFIG_REPO=$1
CONFIG_FILE=$2
TREE_NAME=$3

export PYTHONPATH=$MOZSEARCH_PATH/scripts
export RUST_BACKTRACE=full

date

$CONFIG_REPO/$TREE_NAME/find-repo-files $CONFIG_FILE $TREE_NAME
$MOZSEARCH_PATH/scripts/mkdirs.sh

date

$MOZSEARCH_PATH/scripts/build.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME

date

export RUST_LOG=info

$MOZSEARCH_PATH/scripts/rust-analyze.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/find-objdir-files.py
$MOZSEARCH_PATH/scripts/objdir-mkdirs.sh

date

$MOZSEARCH_PATH/scripts/js-analyze.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/idl-analyze.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/ipdl-analyze.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/crossref.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/output.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/build-codesearch.py $CONFIG_FILE $TREE_NAME

date
