#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Remove the analysis dir and any platform variations.
rm -rf $INDEX_ROOT/analysis*
# Remove any objdir variants; in general we expect the "objdir" itself to
# potentially have been created during the "setup" step, or at least that's the
# case for m-c.
rm -rf $INDEX_ROOT/objdir-*
rm -rf $INDEX_ROOT/generated-*
rm -rf $INDEX_ROOT/file
rm -rf $INDEX_ROOT/dir
rm -rf $INDEX_ROOT/description
rm -rf $INDEX_ROOT/templates
rm -rf $INDEX_ROOT/aliases

mkdir -p $INDEX_ROOT/analysis
mkdir -p $INDEX_ROOT/file
mkdir -p $INDEX_ROOT/dir
mkdir -p $INDEX_ROOT/description
mkdir -p $INDEX_ROOT/aliases

mkdir -p $INDEX_ROOT/analysis/__GENERATED__

cat $INDEX_ROOT/repo-dirs | \
    parallel \
        "echo 'Making {file,dir,analysis,description} dirs for' {}; \
         cd $INDEX_ROOT/file; mkdir -p {}; \
         cd $INDEX_ROOT/dir/; mkdir -p {}; \
         cd $INDEX_ROOT/analysis; mkdir -p {}; \
         cd $INDEX_ROOT/description; mkdir -p {}"

mkdir -p $INDEX_ROOT/templates

