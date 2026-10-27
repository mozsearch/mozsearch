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
# (see tools/src/bin/dataflow-summarize.rs), on a thread per CPU: its
# scheduling keeps the files being summarized within a memory budget (by the
# machine's memory), so more threads don't need more memory (it ran at 16
# threads on the 32 vCPUs of the config8 indexer for 2 minutes, ~4.7 GiB).
# crossref's 16 threads stay: each thread's malloc arena keeps its biggest
# chunk's memory, and crossref's peak is ~60 GiB.
if [ "$(jq -r ".trees[\"${TREE_NAME}\"].dataflow_summaries // false" ${CONFIG_FILE})" = "true" ]; then
  dataflow-summarize $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH $(nproc)
fi

crossref $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH 16
