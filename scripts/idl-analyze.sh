#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

FILTER=$1
if [ "x${FILTER}" = "x" ]
then
  FILTER=".*"
fi

PYTHON=$OBJDIR/_virtualenv/bin/python

export PYTHONPATH=$TREE_ROOT/xpcom/idl-parser/xpidl

cat $INDEX_ROOT/idl-files | grep "$FILTER" | \
    parallel $PYTHON $MOZSEARCH_ROOT/idl-analyze.py \
    $INDEX_ROOT $TREE_ROOT/{} ">" $INDEX_ROOT/analysis/{}
echo $?
