#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

pushd ${INDEX_ROOT}/analysis/__GENERATED__
find . -type f | sed -e 's#^./#__GENERATED__/#' > ${INDEX_ROOT}/objdir-files
find . -mindepth 1 -type d | sed -e 's#^./#__GENERATED__/#' > ${INDEX_ROOT}/objdir-dirs
popd

# This is shuffled for the benefit of the "parallel" invocation in output.sh so that
# file complexity is (more) randomly distributed.  crossref.rs also now ingests
# this file too, but it doesn't inherently need the shuffling.  When we stop using
# "parallel" for output.sh, we can remove the "shuf" invocation.
cat ${INDEX_ROOT}/repo-files ${INDEX_ROOT}/objdir-files | shuf > ${INDEX_ROOT}/all-files
# This is being created for crossref.rs right now and we're not shuffling because
# we don't need to.
cat ${INDEX_ROOT}/repo-dirs ${INDEX_ROOT}/objdir-dirs > ${INDEX_ROOT}/all-dirs
