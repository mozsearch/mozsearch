#!/usr/bin/env bash

# Upload the whole indexer log to the indexer-logs bucket as
# "KIND-DATE_NAME.gz" (ex: "reblame-2026-09-28T21:22+00:00_dev-history_just-fd.gz")
# and print the key.  The log is ~ubuntu/index-log, which main.sh makes a
# symlink to the log on the instance's SSD, which is lost when the instance
# shuts down, so everything which shuts it down uploads the log first.
#
# Usage: upload-log.sh KIND NAME

set -eu
set -o pipefail

if [ $# != 2 ]; then
    echo "usage: $0 <kind> <name>" > /dev/stderr
    exit 1
fi

AWS_ROOT=$(dirname "$(readlink -f "$0")")
LOG=$(readlink -f ~ubuntu/index-log)
KEY="$1-$(date -Iminutes)_$2.gz"

# (gzip won't compress a symlink, hence the readlink.)
gzip -kf "$LOG"
"$AWS_ROOT/upload.py" "$LOG.gz" indexer-logs "$KEY"
echo "$KEY"
