#!/usr/bin/env python3

# Record a short status message in this instance's "status" tag (and the time in
# its "status-updated" tag), so that progress can be checked without ssh-ing in
# (ex: with reblame-status.py).  This is best effort: failures are reported but
# never fail the caller.
#
# Usage: set-status.py <message...>

from datetime import datetime, timezone
import subprocess
import sys

import boto3

# The maximum length of an EC2 tag value.
MAX_TAG_VALUE = 256

message = " ".join(sys.argv[1:])[:MAX_TAG_VALUE]
try:
    instance_id = subprocess.check_output(["ec2metadata", "--instance-id"], text=True).strip()
    boto3.client("ec2").create_tags(
        Resources=[instance_id],
        Tags=[
            {"Key": "status", "Value": message},
            {
                "Key": "status-updated",
                "Value": datetime.now(timezone.utc).isoformat(timespec="seconds"),
            },
        ],
    )
except Exception as e:
    print(f"set-status.py: Unable to record status {message!r}: {e}", file=sys.stderr)
