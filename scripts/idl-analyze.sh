#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

FILTER=$1
if [ "x${FILTER}" = "x" ]
then
  FILTER=".*"
fi

rm -rf /tmp/pymodules
mkdir /tmp/pymodules
pushd /tmp/pymodules
wget "http://hg.mozilla.org/mozilla-central/raw-file/tip/xpcom/idl-parser/xpidl/xpidl.py"
mkdir ply
pushd ply
wget "http://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/__init__.py"
wget "http://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/lex.py"
wget "http://hg.mozilla.org/mozilla-central/raw-file/tip/other-licenses/ply/ply/yacc.py"
popd
popd

export PYTHONPATH=/tmp/pymodules

cat $INDEX_ROOT/idl-files | grep "$FILTER" | \
    parallel python $MOZSEARCH_PATH/idl-analyze.py \
    $INDEX_ROOT $FILES_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
