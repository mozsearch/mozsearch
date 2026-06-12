#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2
ANALYSIS_FILES_PATH=$3

echo Root is $INDEX_ROOT

crossref $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH 16
