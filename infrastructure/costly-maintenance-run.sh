#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Run the costly-maintenance script from the config repo of each tree which has
# one, as reblame-run.sh does for their reblame scripts.  These do the
# maintenance which needs more time or memory than the daily indexing has to
# spare (ex: fully repacking firefox-disco's history), on an instance of its own
# (see aws/costly-maintenance.sh, which aws/trigger_costly_maintenance.py runs).

if [ $# -lt 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <working-dir> [extra-args-for-costly-maintenance]"
    exit 1
fi

export MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)
export CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"
export WORKING=$(readlink -f $3)

# Remove first three command-line args from the $* variable, so we're just left with the
# "extra arguments" to pass on to the per-repo costly-maintenance script.
shift 3

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $CONFIG_INPUT $WORKING $WORKING
CONFIG_FILE=$WORKING/config.json

MAINTAINED=0
for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
    if [ -f "$CONFIG_REPO/$TREE_NAME/costly-maintenance" ]; then
        . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
        mkdir -p $INDEX_ROOT
        cd $INDEX_ROOT
        $CONFIG_REPO/$TREE_NAME/costly-maintenance $*
        MAINTAINED=$((MAINTAINED + 1))
    fi
done

if [ $MAINTAINED == 0 ]; then
    echo "None of the trees in $CONFIG_INPUT has a costly-maintenance script."
fi
