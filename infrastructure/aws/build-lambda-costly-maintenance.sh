#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Usage: build-lambda-costly-maintenance.sh <mozsearch-repo> <config-repo> <config-file> <branch> [release|dev]
#
# Like build-lambda-indexer-start.sh, but for a lambda job which runs
# trigger_costly_maintenance.py (monthly; see "Costly maintenance" in
# docs/aws.md).

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
cp $MOZSEARCH_PATH/infrastructure/aws/trigger_costly_maintenance.py /tmp/lambda

cat >/tmp/lambda/lambda-costly-maintenance.py <<EOF
#!/usr/bin/env python3

import boto3
import trigger_costly_maintenance

def start(event, context):
    cmd = trigger_costly_maintenance.TriggerCostlyMaintenanceCommand()
    cmd.parse_args(["$MOZSEARCH_REPO", "$CONFIG_REPO", "$CONFIG_INPUT", "$BRANCH", "$CHANNEL"])
    cmd.trigger()
EOF

pushd /tmp/lambda
python3 -m venv env
env/bin/pip install boto3
# See build-lambda-indexer-start.sh.
env/bin/pip install --upgrade certifi
cp -r env/lib/python3*/site-packages/* .
rm -rf env

rm -rf /tmp/lambda.zip
zip -r /tmp/lambda.zip *

popd
rm -rf /tmp/lambda

FINAL_ZIP_NAME=/vagrant/lambda-costly-maintenance-$CHANNEL.zip
mv /tmp/lambda.zip $FINAL_ZIP_NAME
echo "Upload $FINAL_ZIP_NAME to AWS Lambda"
