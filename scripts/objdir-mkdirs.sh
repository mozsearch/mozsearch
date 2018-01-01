#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

cat $INDEX_ROOT/objdir-dirs | while IFS= read dir
do
  mkdir -p "$INDEX_ROOT/file/$dir"
  mkdir -p "$INDEX_ROOT/dir/$dir"
done
