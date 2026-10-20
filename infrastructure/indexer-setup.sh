#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ -n "${CHECK_WARNINGS:-}" ]; then
    exec > >(tee /tmp/indexer-setup-log) 2>&1
fi

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <index-path>"
    exit 1
fi

export MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)
export CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"
export WORKING=$(readlink -f $3)

if [ -z "${CLEAN_WORKING:-}" ]; then
    echo "Keeping old contents of $WORKING/. Set CLEAN_WORKING=1 to remove the contents of $WORKING/."
else
    echo "Removing old contents of $WORKING/."
    rm -rf $WORKING/*
fi

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $CONFIG_INPUT $WORKING $WORKING
CONFIG_FILE=$WORKING/config.json

setup_tree() {
    TREE_NAME=$1
    echo "Performing indexer-setup section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"

    echo "Performing load-vars step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    . $MOZSEARCH_PATH/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME

    mkdir -p $INDEX_ROOT

    # If this fails, we let it propagate to abort the entire indexer. Setup
    # failure is generally pretty bad and can leave the git repo in a weird
    # state that doesn't lend itself to graceful fallback. See bug 1842632.
    echo "Performing setup step for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
    $CONFIG_REPO/$TREE_NAME/setup

    echo "Performed indexer-setup section for $TREE_NAME : $(date +"%Y-%m-%dT%H:%M:%S%z")"
}

# With "setup_groups" in the config (a list of lists of trees), each group's
# trees are set up one after another, in the group's order, while the other
# groups' are: a tree goes in the group of the trees whose setups it depends on
# or shares anything with (ex: firefox-try's, which uses firefox-disco's
# firefox-shared repos and its target.json), and in a group of its own if its
# setup is independent (ex: wubspecs', which only downloads and updates its own
# repos and history).  Each group's log goes in the main log when the group is
# done.  The trees in no group are set up afterwards, one after another.
SETUP_GROUPS=$(jq -c '.setup_groups // []' ${CONFIG_FILE})
GROUPED_TREES=$(jq -r '.setup_groups // [] | flatten | .[]' ${CONFIG_FILE})
for TREE_NAME in $GROUPED_TREES; do
    if ! jq -e --arg tree "$TREE_NAME" '.trees | has($tree)' ${CONFIG_FILE} > /dev/null; then
        echo "setup_groups has $TREE_NAME, which isn't a tree."
        exit 1
    fi
done
if [[ $(echo "$GROUPED_TREES" | sort | uniq -d) != "" ]]; then
    echo "setup_groups has trees in more than one group: $(echo "$GROUPED_TREES" | sort | uniq -d)"
    exit 1
fi

GROUP_COUNT=$(echo "$SETUP_GROUPS" | jq 'length')
if (( GROUP_COUNT > 0 )); then
    mkdir -p $WORKING/tmp
    GROUP_PIDS=()
    for (( GROUP_INDEX = 0; GROUP_INDEX < GROUP_COUNT; GROUP_INDEX++ )); do
        (
            for TREE_NAME in $(echo "$SETUP_GROUPS" | jq -r ".[$GROUP_INDEX][]"); do
                setup_tree $TREE_NAME
            done
        ) > $WORKING/tmp/setup-group-$GROUP_INDEX.log 2>&1 &
        GROUP_PIDS+=($!)
    done
    FAILED_GROUPS=0
    for (( GROUP_INDEX = 0; GROUP_INDEX < GROUP_COUNT; GROUP_INDEX++ )); do
        GROUP_STATUS=0
        wait ${GROUP_PIDS[$GROUP_INDEX]} || GROUP_STATUS=$?
        echo "Setup log for group $GROUP_INDEX ($(echo "$SETUP_GROUPS" | jq -r ".[$GROUP_INDEX] | join(\", \")")):"
        cat $WORKING/tmp/setup-group-$GROUP_INDEX.log
        if [[ $GROUP_STATUS != 0 ]]; then
            echo "Setup group $GROUP_INDEX failed ($GROUP_STATUS)."
            FAILED_GROUPS=$((FAILED_GROUPS + 1))
        fi
    done
    # (As a tree's setup's failure stops the whole indexer: once all the
    # groups are done, so their logs are all in the main log.)
    if (( FAILED_GROUPS > 0 )); then
        exit 1
    fi
fi

for TREE_NAME in $(jq -r ".trees|keys_unsorted|.[]" ${CONFIG_FILE})
do
    if echo "$GROUPED_TREES" | grep -qxF "$TREE_NAME"; then
        continue
    fi
    setup_tree $TREE_NAME
done

if [ -n "${CHECK_WARNINGS:-}" ]; then
    $MOZSEARCH_PATH/infrastructure/aws/send-warning-email.py \
        $MOZSEARCH_PATH/infrastructure/aws/warning-suppression.patterns \
        check-warnings \
        test-error \
        /tmp/indexer-setup-log
fi
