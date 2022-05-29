#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

pushd ${INDEX_ROOT}/analysis/__GENERATED__
find . -type f | sed -e 's#^./#__GENERATED__/#' > ${INDEX_ROOT}/objdir-files
find . -mindepth 1 -type d | sed -e 's#^./#__GENERATED__/#' > ${INDEX_ROOT}/objdir-dirs
popd

cat ${INDEX_ROOT}/repo-files ${INDEX_ROOT}/objdir-files > ${INDEX_ROOT}/all-files
