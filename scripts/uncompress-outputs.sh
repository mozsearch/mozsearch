#!/usr/bin/env bash

set -x # Show commands (parallel does all the heavy lifting, so not spammy)
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

function uncompress_dir {
  echo "Uncompressing files in $1"
  pushd $1
  find . -type f -name '*.gz' | sed -e 's/\.gz$//' | \
      parallel --halt now,fail=1 'if [[ -f {} ]]; then gunzip -f {}.gz; fi'
  popd
}

uncompress_dir "${INDEX_ROOT}/dir/"
uncompress_dir "${INDEX_ROOT}/analysis/"
