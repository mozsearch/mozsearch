#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file> <tree-name>"
    exit 1
fi

CONFIG_REPO=$1
CONFIG_FILE=$2
TREE_NAME=$3

export PYTHONPATH=$MOZSEARCH_PATH/scripts

date

$CONFIG_REPO/$TREE_NAME/find-repo-files $CONFIG_FILE $TREE_NAME
$MOZSEARCH_PATH/scripts/mkdirs.sh

date

$MOZSEARCH_PATH/scripts/build.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_PATH/scripts/find-objdir-files.py
$MOZSEARCH_PATH/scripts/objdir-mkdirs.sh

date

$MOZSEARCH_PATH/scripts/rust-analyze.sh $CONFIG_FILE $TREE_NAME

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
