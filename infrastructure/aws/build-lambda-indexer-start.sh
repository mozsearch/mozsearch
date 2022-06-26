#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Usage: build-lambda-indexer-start.sh <mozsearch-repo> <config-repo> <config-file> <branch> [release|dev]

if [ $# != 5 ]
then
    echo "usage: $0 <mozsearch-repo> <config-repo> <config-file> <branch> <channel (dev or release)>"
    exit 1
fi

MOZSEARCH_REPO=$1
CONFIG_REPO=$2
CONFIG_INPUT=$3
BRANCH=$4
CHANNEL=$5

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/../..)

rm -rf /tmp/lambda
mkdir /tmp/lambda
cp $MOZSEARCH_PATH/infrastructure/aws/trigger_common.py /tmp/lambda
cp $MOZSEARCH_PATH/infrastructure/aws/trigger_indexer.py /tmp/lambda

cat >/tmp/lambda/lambda-indexer-start.py <<EOF
#!/usr/bin/env python3

import boto3
import trigger_indexer

def start(event, context):
    cmd = trigger_indexer.TriggerIndexerCommand()
    cmd.parse_args(["$MOZSEARCH_REPO", "$CONFIG_REPO", "$CONFIG_INPUT", "$BRANCH", "$CHANNEL"])
    cmd.trigger()
EOF

pushd /tmp/lambda
virtualenv --python=python3 env
env/bin/pip install boto3
# Because our virtualenv doesn't specify --no-seed/--without-pip, it may pull
# packages from your machine into the virtualenv which can include a potentially
# out-of-date version of certifi.  For example, on asuth's Ubuntu 20.04 machine,
# certifi==2019.11.28 is installed via a debian package somehow.  Without an
# explicit upgrade, we end up trying to use that version which will fail to
# validate the AWS server's certificate.
env/bin/pip install --upgrade certifi
cp -r env/lib/python3*/site-packages/* .
rm -rf env

rm -rf /tmp/lambda.zip
zip -r /tmp/lambda.zip *

popd
rm -rf /tmp/lambda

FINAL_ZIP_NAME=/vagrant/lambda-$CHANNEL.zip
mv /tmp/lambda.zip $FINAL_ZIP_NAME
echo "Upload $FINAL_ZIP_NAME to AWS Lambda"
