#!/usr/bin/env bash

# Don't set -e here, because if index.sh returns non-zero we want to detect
# that explicitly, which -e will not allow

exec &> /home/ubuntu/index-log

set -x # Show commands
set -u # Undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# See index.sh for the arguments to this script

SELF=$(readlink -f "$0")
BRANCH=$1
CHANNEL=$2
AWS_ROOT=$(dirname "$SELF")

mkdir -p ~/.aws
cat > ~/.aws/config <<"STOP"
[default]
region = us-west-2
STOP

EMAIL_PREFIX="${CHANNEL}/${BRANCH}"
case "${CHANNEL}" in
    release | mozilla-releases )
        DEST_EMAIL="searchfox-aws@mozilla.com"
        ;;
    * )
        DEST_EMAIL=$(git --git-dir="${AWS_ROOT}/../../.git" show --format="%aE" --no-patch HEAD)
        ;;
esac

# Create a crontab entry to send failure email if indexing takes too long. This
# is basically a failsafe for if this indexer instance doesn't shut down within
# 6 hours.
$AWS_ROOT/make-crontab.py "[${EMAIL_PREFIX}/timeout]" "${DEST_EMAIL}"

# Run indexer with arguments supplied to this script; if it fails then send
# failure email and shut down. Release channel failures get sent to the default
# email address, other channel failures get sent to the author of the head
# commit.
$AWS_ROOT/index.sh $*
if [ $? -ne 0 ]; then
    # In the event of failure, we will have byproducts leftover on the local
    # drive that will be lost if we don't first move them to the persistent EBS
    # store.  We create an "interrupted" parent directory for these contents in
    # order to avoid any ambiguities about what the state of the scratch drive
    # was.
    mkdir /index/interrupted
    mv -f /mnt/index-scratch/* /index/interrupted

    $AWS_ROOT/send-failure-email.py "[${EMAIL_PREFIX}]" "${DEST_EMAIL}"
fi
