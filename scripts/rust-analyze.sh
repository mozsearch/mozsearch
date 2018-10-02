#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: rust-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

if [ -d "$OBJDIR" ]; then
  ANALYSIS_DIRS="$(find $OBJDIR -type d -name save-analysis)"
  if [ "x$ANALYSIS_DIRS" != "x" ]; then
    $MOZSEARCH_PATH/tools/target/release/rust-indexer \
      "$FILES_ROOT" \
      "$INDEX_ROOT/analysis" \
      "$OBJDIR" \
      $ANALYSIS_DIRS
  fi
fi
