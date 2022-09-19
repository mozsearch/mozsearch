#!/usr/bin/env bash

# This script:
# 1. Locates the rust save-analysis directories under the provided root.
# 2. Invokes rust-indexer with those analysis directories and provides a number
#    of path prefixes to help map file paths to searchfox's special
#    __GENERATED__ prefix.

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -lt 5 ]
then
    echo "Usage: rust-analyze.sh config-file.json tree_name rust_analysis_in generated_src sf_analysis_out"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2
# This is where we find the save-analysis files.  For mozilla-central builds where
# we have multiple platform-specific objdirs that are processed in parallel,
# we expect this to be objdir-$PLATFORM.  For self-built single-platform cases,
# this will be the objdir.
RUST_ANALYSIS_IN=$3
# This is where we find the source code corresponding to __GENERATED__ files.
# This is the objdir in self-built single-platform cases and generated-$PLATFORM
# in multi-platform cases at the current time.
GENERATED_SRC=$4
# This is where we write the resulting searchfox analysis files.  We expect
# this to be a platform-specific directory like analysis-$PLATFORM in
# multi-platform cases (which will be processed by merge-analyses) and analysis
# in single-platform cases.
SF_ANALYSIS_OUT=$5

if [ -d "$RUST_ANALYSIS_IN" ]; then
  INPUTS="$(find $RUST_ANALYSIS_IN -type d -name save-analysis)"
  SCIP_FLAGS=""
  if [ "x$INPUTS" = "x" ]; then
    INPUTS="$(find $RUST_ANALYSIS_IN -type f -name rust.scip)"
    SCIP_FLAGS="--scip --scip-prefix $RUST_ANALYSIS_IN"
  else
    # Rust stdlib files use `analysis` directories instead of `save-analysis`, so
    # even though they live under the same root, it needs a separate find pass
    # because the above will not have found them.
    #
    # Note that we also only expect a rustlib in gecko indexing jobs.
    if [ -d "$RUST_ANALYSIS_IN/rustlib" ]; then
      INPUTS="$INPUTS $(find $RUST_ANALYSIS_IN/rustlib -type d -name analysis)"
    fi
  fi

  if [ "x$INPUTS" = "x" ]; then
    exit 0 # Nothing to analyze really
  fi

  $MOZSEARCH_PATH/tools/target/release/rust-indexer \
    "$FILES_ROOT" \
    "$SF_ANALYSIS_OUT" \
    "$GENERATED_SRC" \
    $SCIP_FLAGS \
    $INPUTS
fi
