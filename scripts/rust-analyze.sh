#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -lt 2 ]
then
    echo "Usage: rust-analyze.sh config-file.json tree_name [platform-filter]"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# Our current rust-indexer implementation can get confused if it's given the
# same crate but for different platforms at the same time.  So we provide an
# optional mechanism that takes a platform name per mozsearch-mozilla
# convention and:
# - filters the `find` invocations to only pick up output for that platform.
# - puts the output in `analysis-linux64` for example instead of just
#   `analysis`.
PLATFORM=${3:-}

# It's possible the rust code was built on a path structure that doesn't match
# the one we're using locally on this machine.  In that case, rust-indexer needs
# to be told what that path prefix is so it can be stripped off.  We still tell
# it FILES_ROOT as the first argument as the `src_dir` in all cases because it
# also needs to be able to map from relative paths to actual paths on disk.
#
# If the BUILT_DIR is not provided, we fall back to using the FILES_ROOT as the
# built directory.
USE_AS_BUILT_DIR=${BUILT_DIR:-$FILES_ROOT}

# Figure out the name of the platform specific dir used for the rust stuff.
declare -A RUST_PLAT_DIRS
RUST_PLAT_DIRS["linux64"]="x86_64-unknown-linux-gnu"
RUST_PLAT_DIRS["macosx64"]="x86_64-apple-darwin"
RUST_PLAT_DIRS["win64"]="x86_64-pc-windows-msvc"
RUST_PLAT_DIRS["android-armv7"]="thumbv7neon-linux-androideabi"

# Hacky mechanism where we take a rust objdir value like x86_64-unknown-linux-gnu
# to filter the rust analysis to just that.
PLATFORM_SAVE_FILTER=${PLATFORM:+${RUST_PLAT_DIRS[$PLATFORM]}/debug/deps/}
PLATFORM_FILTER=${PLATFORM:+${RUST_PLAT_DIRS[$PLATFORM]}/}
PLATFORM_SUFFIX=${PLATFORM:+-$PLATFORM}

if [ -d "$OBJDIR" ]; then
  # Bail if the build step already performed rust analysis.
  if [ -f "$OBJDIR/rust-analyzed" ]; then
    exit 0
  fi

  ANALYSIS_DIRS="$(find $OBJDIR -type d -path \*/${PLATFORM_SAVE_FILTER}save-analysis)"
  if [ "x$ANALYSIS_DIRS" = "x" ]; then
    exit 0 # Nothing to analyze really.
  fi
  # If we have rust stdlib sources and analysis data, pick that up too
  if [ -d "$INDEX_ROOT/rustlib" ]; then
    ANALYSIS_DIRS="$ANALYSIS_DIRS $(find $INDEX_ROOT/rustlib -type d -path \*/${PLATFORM_FILTER}analysis)"
  fi
  $MOZSEARCH_PATH/tools/target/release/rust-indexer \
    "$FILES_ROOT" \
    "$INDEX_ROOT/analysis${PLATFORM_SUFFIX}" \
    "$OBJDIR" \
    "$INDEX_ROOT/rustlib/src/rust/src" \
    "$USE_AS_BUILT_DIR" \
    $ANALYSIS_DIRS
fi
