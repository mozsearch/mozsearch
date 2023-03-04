#!/usr/bin/env bash

# This script locates all .scip files in the OBJDIR and then generates analysis
# data from the scip indices.
#
# This data-flow path is evolved from the WIP rust SCIP analysis functionality,
# and changes are still in flight.

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -lt 5 ]
then
    echo "Usage: scip-analyze.sh config-file.json tree_name scip_analysis_in generated_src sf_analysis_out"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2
# This is where we find the SCIP files.  For mozilla-central builds where
# we have multiple platform-specific objdirs that are processed in parallel,
# we expect this to be objdir-$PLATFORM.  For self-built single-platform cases,
# this will be the objdir.
SCIP_ANALYSIS_IN=$3
# This is where we find the source code corresponding to __GENERATED__ files.
# This is the objdir in self-built single-platform cases and generated-$PLATFORM
# in multi-platform cases at the current time.
GENERATED_SRC=$4
# This is where we write the resulting searchfox analysis files.  We expect
# this to be a platform-specific directory like analysis-$PLATFORM in
# multi-platform cases (which will be processed by merge-analyses) and analysis
# in single-platform cases.
SF_ANALYSIS_OUT=$5

if [ -d "$SCIP_ANALYSIS_IN" ]; then
  INPUTS="$(find $SCIP_ANALYSIS_IN -type f -name '*.scip')"
  SCIP_FLAGS="--scip-prefix $SCIP_ANALYSIS_IN"

  if [ "x$INPUTS" = "x" ]; then
    exit 0 # Nothing to analyze really
  fi

  $MOZSEARCH_PATH/tools/target/release/scip-indexer \
    "$FILES_ROOT" \
    "$SF_ANALYSIS_OUT" \
    "$GENERATED_SRC" \
    $SCIP_FLAGS \
    $INPUTS
fi
