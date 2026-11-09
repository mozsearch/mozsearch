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
#
# dataflow-summarize runs alongside crossref, which only needs its outputs
# (the summaries and lock regions) after reading the analysis, and waits for
# its exit status in DATAFLOW_SUMMARIZE_STATUS then: crossref's first phases
# (loading, ingesting and reading the analysis, 3.2 minutes for firefox-disco
# on config8's 2026-11-09 run) left most of the cores idle while
# dataflow-summarize (136 s) ran before them.
DATAFLOW_PID=
if [ "$(jq -r ".trees[\"${TREE_NAME}\"].dataflow_summaries // false" ${CONFIG_FILE})" = "true" ]; then
  export DATAFLOW_SUMMARIZE_STATUS=$INDEX_ROOT/dataflow-summarize.status
  rm -f "$DATAFLOW_SUMMARIZE_STATUS"
  (
    STATUS=0
    dataflow-summarize $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH $(( $(nproc) > 1 ? $(nproc) / 2 : 1 )) ||
      STATUS=$?
    # (Written whole, for crossref to read.)
    echo "$STATUS" > "$DATAFLOW_SUMMARIZE_STATUS.tmp"
    mv "$DATAFLOW_SUMMARIZE_STATUS.tmp" "$DATAFLOW_SUMMARIZE_STATUS"
  ) &
  DATAFLOW_PID=$!
  # (If crossref fails first, dataflow-summarize, the subshell's child,
  # needn't go on.)
  trap '[ -n "$DATAFLOW_PID" ] && pkill -P "$DATAFLOW_PID" 2> /dev/null; true' EXIT
fi

crossref $CONFIG_FILE $TREE_NAME $ANALYSIS_FILES_PATH 16
if [ -n "$DATAFLOW_PID" ]; then
  wait "$DATAFLOW_PID"
  DATAFLOW_PID=
  STATUS=$(cat "$DATAFLOW_SUMMARIZE_STATUS")
  rm -f "$DATAFLOW_SUMMARIZE_STATUS"
  if [ "$STATUS" != 0 ]; then
    echo "dataflow-summarize failed ($STATUS)"
    exit 1
  fi
fi
