# This file is intentionally not executable, because it should always be sourced
# into a pre-existing shell. MOZSEARCH_PATH should be defined prior to sourcing.
# Arguments are the config.json in the index and the tree for which variables
# are desired

if [ -z $MOZSEARCH_PATH ]
then
    echo "Error: load-vars.sh is being sourced without MOZSEARCH_PATH defined"
    return # leave load-vars.sh without aborting the calling script
fi

export CONFIG_FILE=$1
export TREE_NAME=$2

export INDEX_ROOT=$(jq -r ".trees[\"${TREE_NAME}\"].index_path" ${CONFIG_FILE})
export FILES_ROOT=$(jq -r ".trees[\"${TREE_NAME}\"].files_path" ${CONFIG_FILE})
export OBJDIR=$(jq -r ".trees[\"${TREE_NAME}\"].objdir_path" ${CONFIG_FILE})
export GIT_ROOT=$(jq -r ".trees[\"${TREE_NAME}\"].git_path" ${CONFIG_FILE})
export BLAME_ROOT=$(jq -r ".trees[\"${TREE_NAME}\"].git_blame_path" ${CONFIG_FILE})
