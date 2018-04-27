#!/usr/bin/env bash

# Don't set -e here, because if index.sh returns non-zero we want to detect
# that explicitly, which -e will not allow

set -x

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

# Create a crontab entry to send failure email if indexing takes too long. This
# is basically a failsafe for if this indexer instance doesn't shut down within
# 6 hours.
$AWS_ROOT/make-crontab.py

# Run indexer with arguments supplied to this script; if it fails then send
# failure email and shut down. Release channel failures get sent to the default
# email address, other channel failures get sent to the author of the head
# commit.
$AWS_ROOT/index.sh $*
if [ $? -ne 0 ]; then
    if [ $CHANNEL == release ]
    then
        $AWS_ROOT/send-failure-email.py "[$CHANNEL/$BRANCH]" "searchfox-aws@mozilla.com"
    else
        DEST_EMAIL=$(git --git-dir="$AWS_ROOT/../../.git" show --format="%aE" --no-patch HEAD)
        $AWS_ROOT/send-failure-email.py "[$CHANNEL/$BRANCH]" "$DEST_EMAIL"
    fi
fi
