#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 3 ]
then
    echo "Usage: spec-analyze.sh config-repo-path config-file.json tree_name"
    exit 1
fi

CONFIG_REPO=$1
TREE_NAME=$3

# A tree with web specs lists them in its config directory's specs.toml (ex:
# the wubspecs tree's, which also configures its forest), and their sources get
# the terms they define and their references to them (see
# tools/src/spec_analyze).
SPECS_TOML=$CONFIG_REPO/$TREE_NAME/specs.toml
if [ ! -f "$SPECS_TOML" ]; then
    exit 0
fi

spec-analyze $FILES_ROOT $INDEX_ROOT/analysis $SPECS_TOML
