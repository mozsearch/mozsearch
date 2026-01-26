#!/usr/bin/env python3

import boto3

from datetime import datetime, timedelta


def find_old_images(ec2):
    result = ec2.describe_images(
        Owners=['self']
    )

    indexers = []
    web_servers = []

    for image in result['Images']:
        name = image['Name']
        creation_date = datetime.fromisoformat(image['CreationDate'])

        if name.startswith('indexer-'):
            indexers.append({
                'creation_date': creation_date,
                'image': image,
            })
        if name.startswith('web-server-'):
            web_servers.append({
                'creation_date': creation_date,
                'image': image,
            })

    indexers.sort(key=lambda item: item['creation_date'])
    web_servers.sort(key=lambda item: item['creation_date'])

    # Keep two latest images.

    if len(indexers) > 0:
        indexers.pop()
    if len(indexers) > 0:
        indexers.pop()

    if len(web_servers) > 0:
        web_servers.pop()
    if len(web_servers) > 0:
        web_servers.pop()

    return indexers, web_servers


def remove_images(ec2, items):
    launch_threshold = datetime.now() - timedelta(days=10)

    for item in items:
        image = item['image']

        try:
            # If the image has been launched in last 10 days, the image may
            # still be in use.  Skip removing it.
            # NOTE: Given the field is "last launched" time, the field may be
            #       have no value when the image had never been launched.
            #       There seems to be no official documentation that explains
            #       what happens in such case (no property? different string?)
            #       So this block is enclosed with try-except.
            last_launched_time = datetime.fromisoformat(image['LastLaunchedTime'])
            if last_launched_time > launch_threshold:
                continue
        except:
            pass

        image_id = image['ImageId']
        creation_date = image['CreationDate']

        print(f'Removing {image_id}, crearted at {creation_date}...')

        # NOTE: While the document says there's
        #       DeleteAssociatedSnapshots parameter, it doesn't seem to work.
        #       We delete the snapshots below.
        ec2.deregister_image(
            ImageId=image_id,
        )

        for mapping in image['BlockDeviceMappings']:
            if 'Ebs' not in mapping:
                continue
            if 'SnapshotId' not in mapping['Ebs']:
                continue
            snapshot_id = mapping['Ebs']['SnapshotId']

            print(f'Removing {snapshot_id} for {image_id}...')
            ec2.delete_snapshot(
                SnapshotId=snapshot_id,
            )


def start(event, context):
    ec2 = boto3.client('ec2')
    indexers, web_servers = find_old_images(ec2)
    remove_images(ec2, indexers)
    remove_images(ec2, web_servers)


if __name__ == '__main__':
    start(None, None)
