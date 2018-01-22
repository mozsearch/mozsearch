#!/bin/bash

if [ $# -ne 3 -a $# -ne 4 ]
then
    echo "Usage: build.sh config-repo config-file.json tree_name [file_filter]"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_REPO=$(realpath $1)
CONFIG_FILE=$(realpath $2)
TREE_NAME=$3

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)
. $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

FILTER=$3

$CONFIG_REPO/$TREE_NAME/build $FILTER
