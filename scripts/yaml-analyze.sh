#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: yaml-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

cat $INDEX_ROOT/yaml-files | \
    parallel --jobs 8 --pipe --halt 2 \
    $MOZSEARCH_PATH/scripts/yaml-analyze.py \
    $INDEX_ROOT $FILES_ROOT $INDEX_ROOT/analysis

echo $?

