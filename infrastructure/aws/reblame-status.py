#!/usr/bin/env python3

# Show the status of blame/history rebuilding ("reblame") instances launched by
# trigger_blame_rebuild.py, without ssh-ing into them: their state and the
# status they record in their "status" tag as they go (see set-status.py; ex:
# "timeline: 1200/5000 revisions (24.0%), 350/min, ETA 11m"), and the most
# recent reblame logs, which are uploaded to S3 when a reblame finishes or
# fails.  (Terminated instances are only listed for about an hour.)
#
# Usage: reblame-status.py [--logs N] [--tail [KEY]]
#   --logs N: How many recent logs to list (default 5).
#   --tail [KEY]: Print the end of the log with the given key (default: the
#     most recent).

import argparse
from datetime import datetime, timezone
import gzip

import boto3

INSTANCE_TAG = 'blame-builder'
LOG_BUCKET = 'indexer-logs'


def age(when, now):
    seconds = int((now - when).total_seconds())
    days, seconds = divmod(seconds, 86400)
    hours, seconds = divmod(seconds, 3600)
    minutes = seconds // 60
    if days:
        return f'{days}d {hours}h'
    if hours:
        return f'{hours}h {minutes}m'
    return f'{minutes}m'


def show_instances(now):
    ec2 = boto3.resource('ec2')
    instances = list(ec2.instances.filter(Filters=[{'Name': 'tag-key', 'Values': [INSTANCE_TAG]}]))
    if not instances:
        print('No reblame instances.')
        return
    for instance in sorted(instances, key=lambda i: i.launch_time):
        tags = {tag['Key']: tag['Value'] for tag in instance.tags or []}
        print(f"{instance.id} {instance.instance_type} {instance.state['Name']}, "
              f"up {age(instance.launch_time, now)}, channel {tags.get('channel')}, "
              f"{tags.get('cfile')} on {tags.get('branch')}")
        status = tags.get('status')
        if status:
            updated = tags.get('status-updated')
            when = ''
            if updated:
                when = f' ({age(datetime.fromisoformat(updated), now)} ago)'
            print(f'  status{when}: {status}')


def recent_logs(s3):
    logs = []
    for prefix in ['reblame-', 'failed-']:
        paginator = s3.get_paginator('list_objects_v2')
        for page in paginator.paginate(Bucket=LOG_BUCKET, Prefix=prefix):
            for obj in page.get('Contents', []):
                # Failure logs are for every kind of run; keep the reblames'.
                if prefix == 'failed-' and 'rebuild-blame' not in obj['Key']:
                    continue
                logs.append(obj)
    return sorted(logs, key=lambda obj: obj['LastModified'], reverse=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--logs', type=int, default=5)
    parser.add_argument('--tail', nargs='?', const='', metavar='KEY')
    args = parser.parse_args()

    now = datetime.now(timezone.utc)
    show_instances(now)

    s3 = boto3.client('s3')
    logs = recent_logs(s3)
    if args.logs:
        print()
        print(f'Recent reblame logs (s3://{LOG_BUCKET}/):' if logs else 'No reblame logs.')
        for obj in logs[:args.logs]:
            print(f"  {obj['Key']} ({obj['Size'] / 1e6:.1f} MB, {age(obj['LastModified'], now)} ago)")

    if args.tail is not None:
        key = args.tail or (logs[0]['Key'] if logs else None)
        if not key:
            return
        body = s3.get_object(Bucket=LOG_BUCKET, Key=key)['Body'].read()
        lines = gzip.decompress(body).decode('utf-8', 'replace').splitlines()
        print()
        print(f'The end of {key}:')
        for line in lines[-40:]:
            print(f'  {line}')


if __name__ == '__main__':
    main()
