#!/usr/bin/env bash

# Helper to download one of the following date ranges' worth of logs to the
# current directory, as given by the argument to give to this script:
# - "yesterday" (default): The logs from just yesterday.
# - "last-full-week": The logs from the last full Sunday-Saturday range.  This
#   is calculated by asking date for "last saturday" (which is never today, even
#   if today is saturday), and then


# Here is an example S3 URI from a log directory:
# s3://searchfox-web-logs/AWSLogs/653057761566/elasticloadbalancing/us-west-2/2023/01/18/

# AFAICT in order to download a specific set of files, we need to use recursive
# and exclude everything and then only include what we want.  The following
# works:

DATE_RANGE=${1:-yesterday}

if [[ "$DATE_RANGE" == "yesterday" ]]; then
  S3_DATE_URI=$(date -u --date='1 days ago' +s3://searchfox-web-logs/AWSLogs/653057761566/elasticloadbalancing/us-west-2/%Y/%m/%d/)
  aws s3 cp ${S3_DATE_URI} . --recursive
elif [[ "$DATE_RANGE" == "last-week" ]]; then
  # latch the saturday to avoid inconsistency if run around the end of the day.
  LAST_SATURDAY=$(date -u --date='last saturday')
  for ((i=0; i<=6; i++)); do
    S3_DATE_URI=$(date -u --date="${LAST_SATURDAY} -${i} days" +s3://searchfox-web-logs/AWSLogs/653057761566/elasticloadbalancing/us-west-2/%Y/%m/%d/)
    aws s3 cp ${S3_DATE_URI} . --recursive
  done
else
  echo "Unrecognized date-range: ${DATE_RANGE}"
  exit 1
fi

gunzip *.gz
