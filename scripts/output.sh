#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 3 ]
then
    echo "Usage: output.sh config_repo config-file.json tree_name"
    exit 1
fi

CONFIG_REPO=$(realpath $1)
CONFIG_FILE=$(realpath $2)
TREE_NAME=$3

# let's put the "parallel" output in a new `diags` directory, as we're still
# seeing really poor output-file performance in bug 1567724.
DIAGS_DIR=$INDEX_ROOT/diags/output
mkdir -p $DIAGS_DIR
# clean up the directory since in the VM this can persist.
rm -f $DIAGS_DIR/*

JOBLOG_PATH=${DIAGS_DIR}/output.joblog
# let's put all the temp files in our diagnostic dir too.
TMPDIR_PATH=${DIAGS_DIR}

# parallel args:
# --files: Place .par files in the ${TMPDIR_PATH} above which is now not
#   actually a temporary directory but instead a path we save so that we can see
#   what the output of the run was.
# --joblog: Emit a joblog that can be used to `--resume` the previous job and
#   also provides us with general performance runtime info
cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files | \
    parallel --files --joblog $JOBLOG_PATH --tmpdir $TMPDIR_PATH --halt 2 -X --eta \
	     $MOZSEARCH_PATH/tools/target/release/output-file $CONFIG_FILE $TREE_NAME

HG_ROOT=$(jq -r ".trees[\"${TREE_NAME}\"].hg_root" ${CONFIG_FILE})
cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files > ${TMPDIR:-/tmp}/dirs
js $MOZSEARCH_PATH/scripts/output-dir.js $FILES_ROOT $INDEX_ROOT "$HG_ROOT" $MOZSEARCH_PATH $OBJDIR $TREE_NAME ${TMPDIR:-/tmp}/dirs

js $MOZSEARCH_PATH/scripts/output-template.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
js $MOZSEARCH_PATH/scripts/output-help.js $CONFIG_REPO/help.html $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
