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
# --pipepart, -a: Pass the filenames to each job on the job's stdin by chopping
#   up the file passed via `-a`.  Compare with `--pipe` which instead divvies
#   inside the parallel perl process and can in theory be a bottleneck.
# --files: Place .par files in the ${TMPDIR_PATH} above which is now not
#   actually a temporary directory but instead a path we save so that we can see
#   what the output of the run was.
# --joblog: Emit a joblog that can be used to `--resume` the previous job and
#   also provides us with general performance runtime info
# --tmpdir: We specify the location of the .par files via this.
# --block: by passing `-1` we indicate each job should get 1 block of data, with
#   the size of the block basically being (1/nproc * file size).  A value of
#   `-2` would give each job a block half the size and result in twice as many
#   jobs (and therefore twice as much overhead).  The general trade-off reason
#   you might do this is that parallel can detect when a process terminates but
#   not when it's idle.  So to load balance, you potentially would want more
#   jobs, but we're looking at a startup cost of ~15 seconds per process, and
#   we can process about 2000 lines of source per 0.1 second with all 4 cores
#   active, so that suggests we give up about 300kloc's worth of rendering for
#   additional job, which potentially covers a lot of slop.  Also, there's a
#   chance that as some output-file jobs complete earlier, the other jobs may
#   then accelerate as there is reduced contention for (SSD) I/O and spare RAM
#   may increase to allow for writes to be buffered without needing to flush,
#   etc.
# --env RUST_BACKTRACE: propagate the RUST_BACKTRACE environment variable.
parallel --pipepart -a $INDEX_ROOT/all-files --files --joblog $JOBLOG_PATH --tmpdir $TMPDIR_PATH \
    --block -1 --halt 2 --env RUST_BACKTRACE \
    $MOZSEARCH_PATH/tools/target/release/output-file $CONFIG_FILE $TREE_NAME -

# TOOL_CMD="search-files --limit=0 --group-by=dir | batch-render dir"
# SEARCHFOX_SERVER=${CONFIG_FILE} \
#     SEARCHFOX_TREE=${TREE_NAME} \
#     $MOZSEARCH_PATH/tools/target/release/searchfox-tool $TOOL_CMD

HG_ROOT=$(jq -r ".trees[\"${TREE_NAME}\"].hg_root" ${CONFIG_FILE})
cat $INDEX_ROOT/repo-files $INDEX_ROOT/objdir-files > ${TMPDIR:-/tmp}/dirs
js $MOZSEARCH_PATH/scripts/output-dir.js $FILES_ROOT $INDEX_ROOT "$HG_ROOT" $MOZSEARCH_PATH $OBJDIR $TREE_NAME ${TMPDIR:-/tmp}/dirs

js $MOZSEARCH_PATH/scripts/output-template.js $FILES_ROOT $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
js $MOZSEARCH_PATH/scripts/output-help.js $CONFIG_REPO/help.html $INDEX_ROOT $MOZSEARCH_PATH $TREE_NAME
