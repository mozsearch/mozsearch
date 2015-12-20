#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

# Find the files to cross-reference.
cd $INDEX_ROOT/analysis
find . -type f | cut -c 2- > /tmp/files

$JS $MOZSEARCH_ROOT/crossref.js $TREE_ROOT/ $INDEX_ROOT $MOZSEARCH_ROOT /tmp/files
