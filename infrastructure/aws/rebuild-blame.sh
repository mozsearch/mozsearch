#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

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

$MOZSEARCH_PATH/infrastructure/reblame-run.sh $CONFIG_REPO_PATH $CONFIG_INPUT /mnt/index-scratch "--upload"

date
echo "Rebuilding blame complete"

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

gzip -k ~ubuntu/index-log
$AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs "reblame-$(date -Iminutes)_${CHANNEL}_${CONFIG_INPUT%.*}.gz"
$AWS_ROOT/send-done-email.py "[$CHANNEL/$BRANCH]" "$DEST_EMAIL"

# Give logger time to catch up
sleep 30

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)
$AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
