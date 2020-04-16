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

if [ -f "${FILES_ROOT}/xpcom/idl-parser/xpidl/xpidl.py" ]; then
    # The tree we're processing has an xpidl.py, so let's use that
    TREE_PYMODULES="/tmp/pymodules-${TREE_NAME}"
    PULL_FROM_MC=0
else
    # The tree we're processing doesn't have an xpidl.py, so we'll pull m-c's copy with wget
    TREE_PYMODULES="/tmp/pymodules"
    PULL_FROM_MC=1
fi
if [ ! -d "${TREE_PYMODULES}" ]; then
    mkdir "${TREE_PYMODULES}"
    pushd "${TREE_PYMODULES}"
    if [ $PULL_FROM_MC -eq 0 ]; then
        cp "${FILES_ROOT}/xpcom/idl-parser/xpidl/xpidl.py" ./
    else
        wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/xpcom/idl-parser/xpidl/xpidl.py"
    fi
    mkdir ply
    pushd ply
    for PLYFILE in __init__.py lex.py yacc.py; do
        if [ $PULL_FROM_MC -eq 0 ]; then
            cp "${FILES_ROOT}/other-licenses/ply/ply/${PLYFILE}" ./
        else
            wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/${PLYFILE}"
        fi
    done
    popd
    popd
fi

export PYTHONPATH="${TREE_PYMODULES}"

cat $INDEX_ROOT/idl-files | \
    parallel python $MOZSEARCH_PATH/scripts/idl-analyze.py \
    $INDEX_ROOT $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
