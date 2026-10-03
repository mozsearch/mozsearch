#!/usr/bin/env python3

# Upload stdin to s3://BUCKET/KEY, publicly readable, only if the object there
# still has the ETag ETAG (ex: as `aws s3api head-object` reported before it was
# downloaded), so that a job which updates a copy of an object doesn't replace
# a newer one which another job uploaded in the meantime (ex: costly
# maintenance, which repacks a tree's history, while the tree's daily indexing
# updates it; see "Costly maintenance" in docs/aws.md).  S3 checks the ETag,
# atomically, when it completes the (multipart) upload, but we check it first
# too, so as not to upload for nothing.  (`aws s3 cp` can't upload
# conditionally.)
#
# Usage: upload-if-unchanged.py <bucket> <key> <etag>
#
# Exits with status 3, having uploaded nothing, if the object changed (or no
# longer exists).

import collections
from concurrent.futures import ThreadPoolExecutor
import sys

import boto3
from botocore.exceptions import ClientError

# The size of each part but the last: S3 allows 10,000 parts, so this allows
# up to 2.5 TiB.
PART_BYTES = 256 << 20
# How many parts are uploaded at once, each from memory.
THREADS = 8
# Log progress every this many parts.
LOG_PARTS = 40
CHANGED_EXIT_CODE = 3

bucket, key, etag = sys.argv[1:]
# (ETags are quoted, but the quotes are easy to lose.)
etag = '"' + etag.strip('"') + '"'

s3 = boto3.client("s3")


def log(message):
    print(f"upload-if-unchanged.py: {message}", file=sys.stderr, flush=True)


def changed(why):
    log(f"Not uploading s3://{bucket}/{key}, since {why}.")
    sys.exit(CHANGED_EXIT_CODE)


try:
    current = s3.head_object(Bucket=bucket, Key=key)["ETag"]
except ClientError as e:
    if e.response["Error"]["Code"] in ("404", "NoSuchKey"):
        changed("it no longer exists")
    raise
if current != etag:
    changed(f"its ETag is {current}, not {etag}")


# The indexers' python3-boto3 predates CompleteMultipartUpload's IfMatch
# parameter, so we add its header ourselves.
def add_if_match(request, **kwargs):
    request.headers["If-Match"] = etag


s3.meta.events.register("before-sign.s3.CompleteMultipartUpload", add_if_match)

upload_id = s3.create_multipart_upload(Bucket=bucket, Key=key, ACL="public-read")["UploadId"]


def upload_part(number, data):
    response = s3.upload_part(
        Bucket=bucket, Key=key, UploadId=upload_id, PartNumber=number, Body=data
    )
    return {"PartNumber": number, "ETag": response["ETag"]}


try:
    parts = []
    with ThreadPoolExecutor(THREADS) as pool:
        uploading = collections.deque()
        number = 0
        size = 0
        while True:
            # (This only reads less than PART_BYTES at the end of stdin.)
            data = sys.stdin.buffer.read(PART_BYTES)
            if not data and number:
                break
            number += 1
            # Wait for the oldest part once THREADS are uploading, so that at
            # most THREADS + 1 parts are in memory.
            if len(uploading) == THREADS:
                parts.append(uploading.popleft().result())
            uploading.append(pool.submit(upload_part, number, data))
            size += len(data)
            if number % LOG_PARTS == 0:
                log(f"uploading part {number} ({size / 2**30:.1f} GiB so far)")
            if len(data) < PART_BYTES:
                break
        parts.extend(future.result() for future in uploading)
    log(f"uploaded {len(parts)} parts ({size / 2**30:.1f} GiB)")
    s3.complete_multipart_upload(
        Bucket=bucket, Key=key, UploadId=upload_id, MultipartUpload={"Parts": parts}
    )
except ClientError as e:
    s3.abort_multipart_upload(Bucket=bucket, Key=key, UploadId=upload_id)
    # (409 ConditionalRequestConflict means another upload completed while S3
    # completed ours.)
    if e.response["Error"]["Code"] in ("PreconditionFailed", "NoSuchKey", "ConditionalRequestConflict"):
        changed(f"it changed while we uploaded ({e.response['Error']['Code']})")
    raise
except BaseException:
    s3.abort_multipart_upload(Bucket=bucket, Key=key, UploadId=upload_id)
    raise
log(f"uploaded s3://{bucket}/{key}")
