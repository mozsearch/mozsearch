#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

CONFIG_FILE=$1
TREE_NAME=$2

# Find the files to cross-reference.
cd $INDEX_ROOT/analysis
find . -type f | cut -c 3- > /tmp/files

$MOZSEARCH_ROOT/tools/target/release/crossref $CONFIG_FILE $TREE_NAME /tmp/files
