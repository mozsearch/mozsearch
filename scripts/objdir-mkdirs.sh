#!/bin/bash

set -e # Errors are fatal
set +x # Don't show commands!

cat $INDEX_ROOT/objdir-dirs | while IFS= read dir
do
  mkdir -p "$INDEX_ROOT/file/$dir"
  mkdir -p "$INDEX_ROOT/dir/$dir"
done

set -x

