#!/bin/bash

if [ $# -ne 3 ]
then
    echo "Usage: build.sh config-repo config-file.json tree_name"
    exit 1
fi

set -e # Errors are fatal
set -x # Show commands

CONFIG_REPO=$(realpath $1)
CONFIG_FILE=$(realpath $2)
TREE_NAME=$3

$CONFIG_REPO/$TREE_NAME/build
