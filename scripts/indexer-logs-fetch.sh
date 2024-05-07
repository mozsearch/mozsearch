#!/usr/bin/env bash

if [ $(uname) == "Darwin" ]; then
    YESTERDAY=$(date -Idate -v -1d)
else
    YESTERDAY=$(date -Idate --date='1 days ago')
fi

# AFAICT in order to download a specific set of files, we need to use recursive
# and exclude everything and then only include what we want.  The following
# works:
aws s3 cp s3://indexer-logs/ . --recursive --exclude '*' --include "index-${YESTERDAY}*.gz"
# grep -z doesn't work on gzipped things, so de-gzip them
gunzip *.gz

# having 2 indexer1 jobs is bad, delete the UTC22 one
rm index-*T0*_release1_config1
