#!/usr/bin/env bash

set -x # Show commands (parallel does all the heavy lifting, so not spammy)
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

function uncompress_dir {
  echo "Uncompressing files in $1"
  pushd $1
  find . -type f -name '*.gz' | while read GZ_FILE; do
      RAW_FILE=$(echo "$GZ_FILE" | sed -e 's/\.gz$//')
      if [[ -f "$RAW_FILE" ]]; then
          gunzip -f "$GZ_FILE"
      fi
  done
  popd
}

uncompress_dir "${INDEX_ROOT}/dir/"
uncompress_dir "${INDEX_ROOT}/analysis/"
