import boto3

def await_volume(client, volumeId, waitingState, finishedState):
    while True:
        volumes = client.describe_volumes(VolumeIds=[volumeId])
        state = volumes['Volumes'][0]['State']
        if state != waitingState:
            break

    if state != finishedState:
        print 'Unexpected volume state (expected {}): {}'.format(finishedState, volumes)
        sys.exit(1)

def await_instance(client, instanceId, waitingState, finishedState):
    while True:
        instances = client.describe_instances(InstanceIds=[instanceId])
        state = instances['Reservations'][0]['Instances'][0]['State']['Name']
        if waitingState and state != waitingState:
            break
        if state == finishedState:
            break

    if state != finishedState:
        print 'Unexpected instance state (expected {}): {}'.format(finishedState, instances)
        sys.exit(1)
