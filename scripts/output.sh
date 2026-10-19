#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 5 ]
then
    echo "Usage: output.sh config_repo config-file.json tree_name url-map.json"
    exit 1
fi

CONFIG_REPO=$(realpath $1)
CONFIG_FILE=$(realpath $2)
TREE_NAME=$3
URL_MAP_PATH=$4
DOC_TREES_PATH=$5

# let's put the "parallel" output in a new `diags` directory, as we're still
# seeing really poor output-file performance in bug 1567724.
DIAGS_DIR=$INDEX_ROOT/diags/output
mkdir -p $DIAGS_DIR
# clean up the directory since in the VM this can persist.
rm -f $DIAGS_DIR/*

JOBLOG_PATH=${DIAGS_DIR}/output.joblog
# let's put all the temp files in our diagnostic dir too.
TMPDIR_PATH=${DIAGS_DIR}

# How many output-file processes run at once: as many as fit in the memory, at
# ~2 GiB each for firefox (a process's base, and its biggest file's formatting
# at once, which it then gives back; see output-file's `release_freed_memory`),
# and as the cores, in a container (a developer's searchfox container shares
# the machine) in half of the memory and the cores less 2 (see
# parallel-jobs.sh).  (It was 16, as with more on a 28-core machine with
# 64 GiB, ~2 GiB each ate the memory.)
# `OUTPUT_FILE_MIB` overrides the size, `PARALLEL_JOBS` the count.
JOBS=$($MOZSEARCH_PATH/scripts/parallel-jobs.sh ${OUTPUT_FILE_MIB:-2048})

# parallel args:
# --jobs: see above.
# --pipepart, -a: Pass the filenames to each job on the job's stdin by chopping
#   up the file passed via `-a`.  Compare with `--pipe` which instead divvies
#   inside the parallel perl process and can in theory be a bottleneck.
# --files: Place .par files in the ${TMPDIR_PATH} above which is now not
#   actually a temporary directory but instead a path we save so that we can see
#   what the output of the run was.
# --joblog: Emit a joblog that can be used to `--resume` the previous job and
#   also provides us with general performance runtime info
# --tmpdir: We specify the location of the .par files via this.
# --block -4: 4 blocks of the files per job slot, which the slots take as
#   they're free, so that a block with slow files doesn't leave the other slots
#   idle at the end (with 1 per slot, config8's slowest block took twice as
#   long as the others).  A process takes ~2 seconds to start (its config,
#   templates, jumprefs, and its files' per-file info).
# --env RUST_BACKTRACE: propagate the RUST_BACKTRACE environment variable.
# "2>&1": If we don't do this, `--files` seems to just eat the stderr output
#   which is obviously suboptimal.
parallel --jobs $JOBS --pipepart -a $INDEX_ROOT/all-files --files --joblog $JOBLOG_PATH --tmpdir $TMPDIR_PATH \
    --block -4 --halt 2 --env RUST_BACKTRACE \
    "output-file $CONFIG_FILE $TREE_NAME $URL_MAP_PATH $DOC_TREES_PATH - 2>&1"

TOOL_CMD="search-files --limit=0 --include-dirs --group-by=directory | batch-render dir"
SEARCHFOX_SERVER=${CONFIG_FILE} \
    SEARCHFOX_TREE=${TREE_NAME} \
    searchfox-tool "$TOOL_CMD"

TOOL_CMD="render search-template"
SEARCHFOX_SERVER=${CONFIG_FILE} \
    SEARCHFOX_TREE=${TREE_NAME} \
    searchfox-tool "$TOOL_CMD"

if [[ -f $CONFIG_REPO/build-help ]]; then
    $CONFIG_REPO/build-help $CONFIG_REPO $INDEX_ROOT/templates/help-input.html
else
    cp $CONFIG_REPO/help.html $INDEX_ROOT/templates/help-input.html
fi

TOOL_CMD="render help"
SEARCHFOX_SERVER=${CONFIG_FILE} \
    SEARCHFOX_TREE=${TREE_NAME} \
    searchfox-tool "$TOOL_CMD"

TOOL_CMD="render landing"
SEARCHFOX_SERVER=${CONFIG_FILE} \
    SEARCHFOX_TREE=${TREE_NAME} \
    searchfox-tool "$TOOL_CMD"

TOOL_CMD="render settings"
SEARCHFOX_SERVER=${CONFIG_FILE} \
    SEARCHFOX_TREE=${TREE_NAME} \
    searchfox-tool "$TOOL_CMD"

# Firefox's version cuts, for the file timeline (see milestone-cuts.py).
if [ -f "$GIT_ROOT/config/milestone.txt" ]; then
    mkdir -p $INDEX_ROOT/pages
    $MOZSEARCH_PATH/scripts/milestone-cuts.py $GIT_ROOT > $INDEX_ROOT/pages/milestones.json
fi
