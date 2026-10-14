#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: metadata-analyze.sh config-file.json tree_name"
    exit 1
fi

# Files' metadata from the rules the tree declares it by, with the rules'
# lines (moz.build files' Bugzilla components, review-context.toml's rules,
# and test manifests' conditions), for per-file-info.toml's ingestion by
# crossref (see tools/src/bin/metadata-analyze.rs).  A tree without such
# rules gets none of its outputs.
metadata-analyze $FILES_ROOT $INDEX_ROOT
