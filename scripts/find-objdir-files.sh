#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Some repositories don't have objdir.
mkdir -p ${INDEX_ROOT}/objdir

pushd ${INDEX_ROOT}/objdir
# All text-like files in objdir are going to be reflected to the output.
#
# NOTE: exclude some known binary filename extensions to reduce the cost of
#       the file type detection.
set +o pipefail # grep can fail
find . -type f -not -regex "\.(o|out|so|a|so\..*|scip)$" -exec file --mime {} + \
    | grep -v 'charset=binary' \
    | cut -d ":" -f 1 \
    | sed -e 's#^./#__GENERATED__/#' \
    > ${INDEX_ROOT}/objdir-files
set -o pipefail
find . -mindepth 1 -type d \
    | sed -e 's#^./#__GENERATED__/#' \
    > ${INDEX_ROOT}/objdir-dirs
popd

# This is shuffled for the benefit of the "parallel" invocation in output.sh so that
# file complexity is (more) randomly distributed.  crossref.rs also now ingests
# this file too, but it doesn't inherently need the shuffling.  When we stop using
# "parallel" for output.sh, we can remove the "shuf" invocation.
cat ${INDEX_ROOT}/repo-files ${INDEX_ROOT}/objdir-files | shuf > ${INDEX_ROOT}/all-files
# This is being created for crossref.rs right now and we're not shuffling because
# we don't need to.
cat ${INDEX_ROOT}/repo-dirs ${INDEX_ROOT}/objdir-dirs > ${INDEX_ROOT}/all-dirs
# We need __GENERATED__ to exist on its own too, but it won't from the above.
echo __GENERATED__ >> ${INDEX_ROOT}/all-dirs
