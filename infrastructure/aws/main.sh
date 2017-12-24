#!/usr/bin/env bash

# Don't set -e here, because if index.sh returns non-zero we want to detect
# that explicitly, which -e will not allow

set -x

SELF=$(readlink -f "$0")
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
# failure email and shut down
$AWS_ROOT/index.sh $*
if [ $? -ne 0 ]; then
    $AWS_ROOT/send-email.py
fi
