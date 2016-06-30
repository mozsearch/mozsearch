#!/bin/bash

CONFIG_FILE=$1
TREE_NAME=$2

if [ "x$MOZSEARCH_ROOT" == "x" ]
then
    MOZSEARCH_ROOT=$(pwd)
fi

export MOZSEARCH_ROOT

export INDEX_ROOT=$($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos/$TREE_NAME/index_path)
export TREE_ROOT=$($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos/$TREE_NAME/repo_path)
export HG_ROOT=$($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos/$TREE_NAME/hg_path)
export BLAME_ROOT=$($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos/$TREE_NAME/blame_repo_path)
export OBJDIR=$($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos/$TREE_NAME/objdir_path)
