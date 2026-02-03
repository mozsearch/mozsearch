#!/usr/bin/env bash

# Create a backup of a key.
# This allows copying things larger than 5GB, which is not supproted on the
# S3 web UI.

if [ $# == 0 ]
then
    echo "usage: $0 <key> [<bucket>]"
    exit 1
fi

KEY=$1
BUCKET=${2:-searchfox.repositories}

aws s3 cp s3://$BUCKET/$KEY s3://$BUCKET/backups/$KEY
