#!/usr/bin/env bash

# This script is intended to be run outside the VM with AWS creds active after
# running `build-lambda-zips-from-inside-vm.sh`.  There should be 5
# `lambda-releaseN.zip` files in the root of the MOZSEARCH dir and you should
# be running the script from that root.  See `aws.md` for more details.

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline


aws lambda update-function-code \
  --function-name start-release1-indexer \
  --zip-file fileb://lambda-release1.zip
rm lambda-release1.zip

aws lambda update-function-code \
  --function-name start-release2-indexer \
  --zip-file fileb://lambda-release2.zip
rm lambda-release2.zip

aws lambda update-function-code \
  --function-name start-release3-indexer \
  --zip-file fileb://lambda-release3.zip
rm lambda-release3.zip

aws lambda update-function-code \
  --function-name start-release4-indexer \
  --zip-file fileb://lambda-release4.zip
rm lambda-release4.zip

aws lambda update-function-code \
  --function-name start-release5-indexer \
  --zip-file fileb://lambda-release5.zip
rm lambda-release5.zip

aws lambda update-function-code \
  --function-name start-release6-indexer \
  --zip-file fileb://lambda-release6.zip
rm lambda-release6.zip
