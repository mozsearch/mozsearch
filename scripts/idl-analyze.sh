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

# Use xpidl and webidl parsers from the tree itself if available.
# They are unstable and for instance the esr140 and nightly tree disagree on how to define async iterables in WebIDL.
if [ -f "${FILES_ROOT}/xpcom/idl-parser/xpidl/xpidl.py" -a \
     -f "${FILES_ROOT}/dom/bindings/parser/WebIDL.py" ]; then
  TREE_PYMODULES="/tmp/pymodules-${TREE_NAME}"
  mkdir -p "${TREE_PYMODULES}"
  pushd "${TREE_PYMODULES}"
  cp "${FILES_ROOT}/xpcom/idl-parser/xpidl/xpidl.py" ./
  cp "${FILES_ROOT}/dom/bindings/parser/WebIDL.py" ./
  mkdir -p ply
  pushd ply
  for PLYFILE in __init__.py lex.py yacc.py; do
    cp "${FILES_ROOT}/other-licenses/ply/ply/${PLYFILE}" ./ || cp "${FILES_ROOT}/third_party/python/ply/ply/${PLYFILE}" ./
  done
  popd
  popd
  export PYTHONPATH="${TREE_PYMODULES}${PYTHONPATH:+:${PYTHONPATH}}"
fi

cat $INDEX_ROOT/idl-files | \
    parallel $MOZSEARCH_PATH/scripts/idl-analyze.py \
    $INDEX_ROOT $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}

cat $INDEX_ROOT/webidl-files | \
    $MOZSEARCH_PATH/scripts/webidl-analyze.py \
    $INDEX_ROOT $FILES_ROOT $INDEX_ROOT/analysis /tmp \
    $WEBIDL_BINDINGS_LOCAL_PATH
echo $?
