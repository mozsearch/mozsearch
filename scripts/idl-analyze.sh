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

if [ -f "${FILES_ROOT}/xpcom/idl-parser/xpidl/xpidl.py" -a \
     -f "${FILES_ROOT}/dom/bindings/parser/WebIDL.py" ]; then
    # The tree we're processing has IDL parsers, so let's use that
    TREE_PYMODULES="/tmp/pymodules-${TREE_NAME}"
    PULL_FROM_MC=0
else
    # The tree we're processing doesn't have IDL parsers, so we'll pull m-c's copy with wget
    TREE_PYMODULES="/tmp/pymodules"
    PULL_FROM_MC=1
fi

# Delete the temp dir if IDL parsers are older than a day (in minutes to avoid
# quantization weirdness).  We'll also try and delete the dir if the file just
# doesn't exist, which also means if the directory doesn't exist.  (We could
# have instead done `-mmin +1440` for affirmative confirmation it's old, but
# since our next check is just for the existence of the directory, this is least
# likely to result in weirdness.)
if [ ! "$(find $TREE_PYMODULES/xpidl.py -mmin -1440)" ]; then
    rm -rf $TREE_PYMODULES
fi

# download/copy as needed
if [ ! -d "${TREE_PYMODULES}" ]; then
    mkdir "${TREE_PYMODULES}"
    pushd "${TREE_PYMODULES}"
    if [ $PULL_FROM_MC -eq 0 ]; then
        cp "${FILES_ROOT}/xpcom/idl-parser/xpidl/xpidl.py" ./
        cp "${FILES_ROOT}/dom/bindings/parser/WebIDL.py" ./
    else
        wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/xpcom/idl-parser/xpidl/xpidl.py"
        wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/dom/bindings/parser/WebIDL.py"
    fi
    mkdir ply
    pushd ply
    for PLYFILE in __init__.py lex.py yacc.py; do
        if [ $PULL_FROM_MC -eq 0 ]; then
            cp "${FILES_ROOT}/other-licenses/ply/ply/${PLYFILE}" ./ || cp "${FILES_ROOT}/third_party/python/ply/ply/${PLYFILE}" ./
        else
            wget "https://hg.mozilla.org/mozilla-central/raw-file/tip/third_party/python/ply/ply/${PLYFILE}"
        fi
    done
    popd
    popd
fi

export PYTHONPATH="${TREE_PYMODULES}"

cat $INDEX_ROOT/idl-files | \
    parallel $MOZSEARCH_PATH/scripts/idl-analyze.py \
    $INDEX_ROOT $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}

cat $INDEX_ROOT/webidl-files | \
    $MOZSEARCH_PATH/scripts/webidl-analyze.py \
    $INDEX_ROOT $FILES_ROOT $INDEX_ROOT/analysis /tmp
echo $?
