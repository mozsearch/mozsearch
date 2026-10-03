#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Run the costly-maintenance scripts of the trees in a config file (see
# ../costly-maintenance-run.sh), then upload the log and terminate, as
# rebuild-blame.sh does for reblame.  trigger_costly_maintenance.py launches the
# instance which runs this (via main.sh).

if [ $# != 4 ]
then
    echo "usage: $0 <branch> <channel> <config-repo-path> <config-file-name>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/../..

BRANCH=$1
CHANNEL=$2
CONFIG_REPO_PATH=$(readlink -f $3)
CONFIG_INPUT="$4"

# The trees' costly-maintenance scripts can record more detailed progress the
# same way.
$AWS_ROOT/set-status.py "running costly maintenance for $CONFIG_INPUT"

$MOZSEARCH_PATH/infrastructure/costly-maintenance-run.sh $CONFIG_REPO_PATH $CONFIG_INPUT /index

date
echo "Costly maintenance complete"

case "$CHANNEL" in
release* )
    DEST_EMAIL="searchfox-aws@mozilla.com"
    ;;
* )
    # For dev-channel runs, send emails to the author of the HEAD commit in the
    # repo.
    DEST_EMAIL=$(git --git-dir="$MOZSEARCH_PATH/.git" show --format="%aE" --no-patch HEAD)
    ;;
esac

LOG_KEY=$($AWS_ROOT/upload-log.sh costly-maintenance "${CHANNEL}_${CONFIG_INPUT%.*}")
$AWS_ROOT/send-done-email.py "[$CHANNEL/$BRANCH]" "$DEST_EMAIL"
$AWS_ROOT/set-status.py "done; the log is indexer-logs/$LOG_KEY; terminating"

# Give logger time to catch up
sleep 30

EC2_INSTANCE_ID=$(ec2metadata --instance-id)
$AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
