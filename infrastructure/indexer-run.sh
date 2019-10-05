#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -lt 2 ]
then
    echo "usage: $0 <config-repo-path> <index-path> [permanent-path]"
    exit 1
fi

export MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)
export CONFIG_REPO=$(readlink -f $1)
export WORKING=$(readlink -f $2)
export PERMANENT=${3:+$(readlink -f $3)}

CONFIG_FILE=$WORKING/config.json

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    $MOZSEARCH_PATH/scripts/mkindex.sh $CONFIG_REPO $CONFIG_FILE $TREE_NAME
    # If we were given a permanent path, move the index results there and
    # symlink from the old location to the new location.
    if [ -n "$PERMANENT" ]
    then
      mv $INDEX_ROOT ${INDEX_ROOT/$WORKING/$PERMANENT}
      ln -s ${INDEX_ROOT/$WORKING/$PERMANENT} $INDEX_ROOT
    fi
done
