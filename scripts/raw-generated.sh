#!/usr/bin/env bash

set -x # Show commands (parallel does all the heavy lifting, so not spammy)
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

echo "Compressing files in $INDEX_ROOT with zero-length marker for try_files"
pushd $INDEX_ROOT
cat objdir-files | \
    sed -e 's|^__GENERATED__/||' | \
    parallel --halt now,fail=1 \
    'gzip -c $OBJDIR/{} > raw/__GENERATED__/{}.gz; touch raw/__GENERATED__/{}'
popd
