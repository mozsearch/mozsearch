#!/bin/bash

export MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

export CONFIG_FILE=$1
export TREE_NAME=$2

export INDEX_ROOT=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/index_path)
export FILES_ROOT=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/files_path)
export OBJDIR=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/objdir_path)

export GIT_ROOT=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/git_path)
export BLAME_ROOT=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/git_blame_path)
export HG_ROOT=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees/$TREE_NAME/hg_path)
