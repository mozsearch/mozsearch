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

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_ROOT=$(dirname "$SCRIPT_PATH")/..
. $MOZSEARCH_ROOT/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_ROOT/scripts/find-repo-files.py $CONFIG_FILE $TREE_NAME
$MOZSEARCH_ROOT/scripts/mkdirs.sh

date

$CONFIG_REPO/$TREE_NAME/build

date

$MOZSEARCH_ROOT/scripts/find-objdir-files.py
$MOZSEARCH_ROOT/scripts/objdir-mkdirs.sh

date

$MOZSEARCH_ROOT/scripts/js-analyze.sh

date

$MOZSEARCH_ROOT/scripts/idl-analyze.sh

date

$MOZSEARCH_ROOT/scripts/crossref.sh $CONFIG_FILE $TREE_NAME

date

$MOZSEARCH_ROOT/scripts/output.sh $CONFIG_FILE $TREE_NAME

date
