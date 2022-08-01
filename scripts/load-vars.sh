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
export TREE_ON_ERROR=$(jq -r ".trees[\"${TREE_NAME}\"].on_error" ${CONFIG_FILE})
export TREE_CACHING=$(jq -r ".trees[\"${TREE_NAME}\"].cache" ${CONFIG_FILE})

# Touches the upload inhibiting marker file and returns 1 so a `||` cascade can
# continue in the same stylistic fashion.  Maybe this is weird but I think a
# && having to follow this would also be weird and you'd have to end up reading
# this comment anyways.
inhibit_upload() {
    touch $INDEX_ROOT/INHIBIT_UPLOAD
    return 1
}
export -f inhibit_upload

handle_tree_error() {
    local msg=$1
    echo "warning: Tree error: $msg"
    if [[ $TREE_ON_ERROR == "continue" ]]; then
        return 0
    fi
    return 1
}
export -f handle_tree_error

# We expect the "cache" key to be one of ["everything", "codesearch", "nothing"]
cache_when_everything() {
    local relpath=$1
    if [[ $TREE_CACHING == "everything" ]]; then
        vmtouch -t $INDEX_ROOT/$relpath
    fi
    return 0
}
export -f cache_when_everything

cache_when_codesearch() {
    local relpath=$1
    if [[ $TREE_CACHING != "nothing" ]]; then
        vmtouch -t $INDEX_ROOT/$relpath
    fi
    return 0
}
export -f cache_when_codesearch
