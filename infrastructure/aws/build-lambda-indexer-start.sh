#!/bin/bash

# Usage: build-lambda-indexer-start.sh <mozsearch-repo> <config-repo> <branch> [release|dev]

set -e # Errors are fatal
set -x # Show commands

if [ $# != 4 ]
then
    echo "usage: $0 <mozsearch-repo> <config-repo> <branch> <channel (dev or release)>"
    exit 1
fi

MOZSEARCH_REPO=$1
CONFIG_REPO=$2
BRANCH=$3
CHANNEL=$4

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

mkdir /tmp/lambda
cp $MOZSEARCH_PATH/infrastructure/aws/trigger_indexer.py /tmp/lambda

cat >/tmp/lambda/lambda-indexer-start.py <<EOF
#!/usr/bin/env python

import boto3
import trigger_indexer

def start(event, context):
    trigger_indexer.trigger("$MOZSEARCH_REPO", "$CONFIG_REPO", "$BRANCH", "$CHANNEL", False)
EOF

pushd /tmp/lambda
virtualenv env
env/bin/pip install boto3
cp -r env/lib/python2.7/site-packages/* .
rm -rf env

rm -rf /tmp/lambda.zip
zip -r /tmp/lambda.zip *

popd
rm -rf /tmp/lambda

echo "Upload /tmp/lambda.zip to AWS Lambda"
