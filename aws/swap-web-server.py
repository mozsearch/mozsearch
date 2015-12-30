# Responsibilities:
# - Start the web server instance, tag it as a web server
# - Attach INDEX_VOL to the web server
# - Wait until the web server is ready to serve requests
#   I could just make requests until they succeed
# - Attach the elastic IP to the new web server
# - Shut down any old web servers (not equal to the one I've started)
# - Delete any old EBS index volumes
#
# Usage: swap-web-server.py <indexer-instance-id> <index-volume-id>

import sys
from datetime import datetime
import boto3
import awslib

indexerInstanceId = sys.argv[1]
volumeId = sys.argv[1]

instances = ec2.instances.filter(InstanceIds=[indexerInstanceId])
indexerInstance = list(instances)[0]

ELASTIC_IP = '52.32.131.4'

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

# - Start the web server instance, tag it as a web server

print 'Starting web server instance...'

userData = open('web-server-startup.sh').read()

r = client.run_instances(
    ImageId='ami-5189a661', # Ubuntu 14.04
    MinCount=1,
    MaxCount=1,
    KeyName='Main Key Pair',
    SecurityGroups=['web-server'],
    UserData=userData,
    InstanceType='t2.medium',
    Placement={'AvailabilityZone': indexerInstance.placement['AvailabilityZone']},
)

webServerInstanceId = r['Instances'][0]['InstanceId']

awslib.await_instance(client, webServerInstanceId, 'pending', 'running')

print '  State is running.'

print 'Tagging web server instance...'

instances = ec2.instances.filter(InstanceIds=[webServerInstanceId])
webServerInstance = list(instances)[0]

client.create_tags(Resources=[webServerInstanceId], Tags=[{
    'Key': 'web-server',
    'Value': str(datetime.now()),
}])

print 'Attaching index volume to web server instance...'

# - Attach INDEX_VOL to the web server
client.attach_volume(VolumeId=volumeId, InstanceId=webServerInstanceId, Device='xvdf')

# - Wait until the web server is ready to serve requests
#   I could just make requests until they succeed

ip = webServerInstance.public_ip_address

#FIXME!!!!!!!

# - Attach the elastic IP to the new web server

print 'Switching elastic IP address...'

client.associate_address(InstanceId=webServerInstanceId, PublicIp=ELASTIC_IP, AllowReassociation=True)

# - Shut down any old web server (a web server not equal to the one I've started)

print 'Shutting down old servers...'

instances = client.describe_instances(Filters=[{'Name': 'tag-key', 'Values': ['web-server']}])
instances = instances['Reservations'][0]['Instances']
terminate = []
for instance in instances:
    instanceId = instance['InstanceId']
    if instanceId != webServerInstanceId:
        terminate.append(instanceId)

print 'Terminating {}'.format(terminate)

client.terminate_instances(InstanceIds=terminate)

for instanceId in terminate:
    awslib.await_instance(client, instanceId, None, 'terminated')

# - Delete any old EBS index volumes

print 'Deleting old EBS index volumes...'

volumes = client.describe_volumes(Filters=[{'Name': 'tag-key', 'Values': ['index']}])
volumes = volumes['Volumes']
for volume in volumes:
    if volumeId != volume['VolumeId']:
        print 'Deleting {}'.format(volume['VolumeId'])
        client.delete_volume(VolumeId=volume['VolumeId'])
