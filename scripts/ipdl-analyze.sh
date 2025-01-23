#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 2 ]
then
    echo "Usage: ipdl-analyze.sh config-file.json tree_name"
    exit 1
fi

CONFIG_FILE=$(realpath $1)
TREE_NAME=$2

# Note that we use "realpath" on FILES_ROOT because the IPDL parser likes to
# canonicalize things.  Because this is just a prefix that gets stripped off,
# all that matters is consistency and so pre-normalizing is fine.
pushd $FILES_ROOT
cat $INDEX_ROOT/ipdl-files | \
    xargs ipdl-analyze $(cat $INDEX_ROOT/ipdl-includes) \
          -f $INDEX_ROOT/repo-files \
          -o $INDEX_ROOT/objdir-files \
          -b $(realpath $FILES_ROOT) \
          -a $INDEX_ROOT/analysis
popd
