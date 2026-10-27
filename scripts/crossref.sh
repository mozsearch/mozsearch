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
# (see tools/src/bin/dataflow-summarize.rs), on a thread per physical core
# (half the vCPUs of the instances' hyperthreaded cores): on the config8
# indexer's 32 vCPUs, firefox-disco's took 122.6 s at 16 threads (88% busy,
# its slowest file 49.7 s) and 198.8 s at 32 (71% busy, 2.6 times the
# threads' time, its slowest file 156.2 s), since threads sharing a core run
# at about half speed, and the slowest file bounds the step.  (Its scheduling
# keeps the files being summarized within a memory budget, by the machine's
# memory, whatever the threads.)  crossref's 16 threads stay: each thread's
# malloc arena keeps its biggest chunk's memory, and crossref's peak is ~60
# GiB.
if [ "$(jq -r ".trees[\"${TREE_NAME}\"].dataflow_summaries // false" ${CONFIG_FILE})" = "true" ]; then
  dataflow-summarize $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH $(( $(nproc) > 1 ? $(nproc) / 2 : 1 ))
fi

crossref $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH 16
