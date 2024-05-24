#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: css-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

cat $INDEX_ROOT/css-files | \
    parallel $MOZSEARCH_PATH/tools/target/release/css-analyze \
    $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
