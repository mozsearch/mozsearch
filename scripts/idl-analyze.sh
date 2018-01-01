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

if [ ! -d /tmp/pymodules ]
then
    mkdir /tmp/pymodules
    pushd /tmp/pymodules
    wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/xpcom/idl-parser/xpidl/xpidl.py"
    mkdir ply
    pushd ply
    wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/__init__.py"
    wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/lex.py"
    wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/yacc.py"
    popd
    popd
fi

export PYTHONPATH=/tmp/pymodules

cat $INDEX_ROOT/idl-files | \
    parallel python $MOZSEARCH_PATH/scripts/idl-analyze.py \
    $INDEX_ROOT $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
