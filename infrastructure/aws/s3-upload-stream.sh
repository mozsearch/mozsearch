#!/usr/bin/env bash

set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Upload stdin to S3 as URL (`aws s3 cp - URL`), a stream of about SIZE bytes
# (ex: an archive's size before compression), with parts big enough for it.
# The aws CLI's CRT transfer client, which the instances' configuration
# chooses (see main.sh), doesn't size a stream's parts by --expected-size, as
# its classic client does: they're its 8 MiB default, and an upload can have
# at most 10,000 parts, so streams stopped at 83.9 GB ("Part number must be an
# integer between 1 and 10000", ex: 2026-11-08's firefox history, 99 GB
# before compression).  So its parts are an 8,000th of SIZE (at least 8 MiB,
# in whole MiB), in a configuration of its own: the CLI's (ex: ~/.aws/config),
# with them as its s3 multipart_chunksize.  The arguments after SIZE are the
# copy's (ex: --acl public-read).
#
# Usage: s3-upload-stream.sh URL SIZE [ARG...] < STREAM

if [ $# -lt 2 ]; then
    echo "Usage: s3-upload-stream.sh URL SIZE [ARG...] < STREAM" 1>&2
    exit 1
fi
URL=$1
SIZE=$2
shift 2

MIB=$((1024 * 1024))
PART_MIB=$(( (SIZE / 8000 + MIB - 1) / MIB ))
if (( PART_MIB < 8 )); then
    PART_MIB=8
fi
CONFIG=$(mktemp)
trap 'rm -f "$CONFIG"' EXIT
CURRENT=${AWS_CONFIG_FILE:-$HOME/.aws/config}
if [ -f "$CURRENT" ]; then
    cp "$CURRENT" "$CONFIG"
fi
# (The active profile's: AWS_PROFILE's, or the default one.)
AWS_CONFIG_FILE=$CONFIG aws configure set s3.multipart_chunksize "${PART_MIB}MB"
echo "Uploading $URL in parts of $PART_MIB MiB (up to $(( PART_MIB * 10000 / 1024 )) GiB)."
AWS_CONFIG_FILE=$CONFIG aws s3 cp - "$URL" --expected-size "$SIZE" "$@"
