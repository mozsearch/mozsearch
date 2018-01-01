#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

echo Root is $INDEX_ROOT

# Find the files to cross-reference.
cd $INDEX_ROOT/analysis
find . -type f | cut -c 3- > /tmp/files
cd -

$MOZSEARCH_PATH/tools/target/release/crossref $CONFIG_FILE $TREE_NAME /tmp/files

ID_FILE=$INDEX_ROOT/identifiers
LC_ALL=C sort -f $ID_FILE > /tmp/ids
mv /tmp/ids $ID_FILE
