from __future__ import absolute_import
from __future__ import print_function
import boto3
import time

def await_volume(client, volumeId, waitingState, finishedState):
    while True:
        volumes = client.describe_volumes(VolumeIds=[volumeId])
        state = volumes['Volumes'][0]['State']
        if state != waitingState:
            break
        time.sleep(1)

    if state != finishedState:
        print('Unexpected volume state (expected {}): {}'.format(finishedState, volumes))
        sys.exit(1)

def await_instance(client, instanceId, waitingState, finishedState):
    exceptionCount = 0
    while True:
        try:
            instances = client.describe_instances(InstanceIds=[instanceId])
            state = instances['Reservations'][0]['Instances'][0]['State']['Name']
            if waitingState and state != waitingState:
                break
            if state == finishedState:
                break
        except:
            if exceptionCount > 2:
                raise
            exceptionCount += 1
            print('Unable to describe instance ID {}, will retry...'.format(instanceId))
            time.sleep(10)
        time.sleep(1)

    if state != finishedState:
        print('Unexpected instance state (expected {}): {}'.format(finishedState, instances))
        sys.exit(1)
