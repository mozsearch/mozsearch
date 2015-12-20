#!/bin/bash

set -e # Errors are fatal
set +x # Don't show commands!

mkdir -p $INDEX_ROOT/analysis
mkdir -p $INDEX_ROOT/file
mkdir -p $INDEX_ROOT/dir

cat $INDEX_ROOT/all-dirs | while IFS= read dir
do
  mkdir -p "$INDEX_ROOT/file/$dir"
  mkdir -p "$INDEX_ROOT/dir/$dir"
  mkdir -p "$INDEX_ROOT/analysis/$dir"
done
mkdir -p $INDEX_ROOT/templates

set -x

