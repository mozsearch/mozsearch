#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2
ANALYSIS_FILES_PATH=$3

echo Root is $INDEX_ROOT

# Trees with `"dataflow_summaries": true` precompute their functions' dataflow
# summaries, which crossref rolls up into classes' and subsystems' data edges
# (see tools/src/bin/dataflow-summarize.rs).
if [ "$(jq -r ".trees[\"${TREE_NAME}\"].dataflow_summaries // false" ${CONFIG_FILE})" = "true" ]; then
  dataflow-summarize $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH 16
fi

crossref $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH 16
