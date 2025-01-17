#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: idl-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

if [[ $(cat $INDEX_ROOT/idl-files $INDEX_ROOT/webidl-files | wc -l) -eq 0 ]]; then
    # If there's no IDL files, bail out so as to avoid unnecessary file downloads
    exit 0
fi

# make xpidl, webidl and ply available
# TODO: remove after next provisioning
PYMODULES="$HOME/pymodules"
export PYTHONPATH="${PYMODULES}${PYTHONPATH:+:${PYTHONPATH}}"

cat $INDEX_ROOT/idl-files | \
    parallel $MOZSEARCH_PATH/scripts/idl-analyze.py \
    $INDEX_ROOT $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}

cat $INDEX_ROOT/webidl-files | \
    $MOZSEARCH_PATH/scripts/webidl-analyze.py \
    $INDEX_ROOT $FILES_ROOT $INDEX_ROOT/analysis /tmp \
    $WEBIDL_BINDINGS_LOCAL_PATH
echo $?
