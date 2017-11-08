# SSH into a server. This command opens the SSH port before connecting
# and closes it after SSH is finished.
#
# Usage:
#   Without arguments, prints a list of instances to connect to.
#   With an instance ID as argument, connects to that instance.

import boto3
import sys
import subprocess
import time

ec2 = boto3.resource('ec2')

def print_instances():
    for instance in ec2.instances.all():
        if len(instance.security_groups) != 1:
            continue

        group = instance.security_groups[0]['GroupName']

        tags = {}
        if instance.tags:
            for tag in instance.tags:
                tags[tag['Key']] = tag['Value']

        print instance.id, group, tags

def change_security(instance, make_secure):
    secure_suffix = '-secure'

    group = instance.security_groups[0]['GroupName']
    if group.endswith(secure_suffix):
        new_group_name = group[:-len(secure_suffix)]
    else:
        new_group_name = group

    if make_secure:
        new_group_name += secure_suffix

    vpc = list(ec2.vpcs.all())[0]
    new_group = vpc.security_groups.filter(GroupNames=[new_group_name])
    new_group = list(new_group)[0]

    print 'Changing instance security group to', new_group.group_name, '--', new_group

    instance.modify_attribute(Groups=[new_group.id])

def log_into(instance):
    change_security(instance, False)

    print 'Connecting to', instance.public_ip_address
    p = subprocess.Popen(['ssh', 'ubuntu@' + instance.public_ip_address])
    p.wait()

    change_security(instance, True)

    sys.exit(p.returncode)

if len(sys.argv) == 1:
    print 'usage: %s <instance-id>' % sys.argv[0]
    print
    print 'Current instances:'
    print_instances()
    sys.exit(0)

id = sys.argv[1]
instance = ec2.Instance(id)
log_into(instance)

