#!/usr/bin/env python3

# SSH into a server. This command opens the SSH port before connecting
# and closes it after SSH is finished.
#
# Usage:
#   Without arguments, prints a list of instances to connect to.
#   With an instance ID as argument, connects to that instance.

from __future__ import absolute_import
from __future__ import print_function
import boto3
from datetime import datetime
import os
import sys
import subprocess
import time

import awslib
from six.moves import input

ec2 = boto3.resource('ec2')
client = boto3.client('ec2')

def print_instances():
    now = None

    for instance in ec2.instances.all():
        if len(instance.security_groups) != 1:
            continue

        state = instance.state['Name']

        group = instance.security_groups[0]['GroupName']

        tags = {}
        if instance.tags:
            for tag in instance.tags:
                tags[tag['Key']] = tag['Value']

        # datetime.now() is timezone-naive which means if we try and subtract
        # to get a timedelta without a tz, we'll get an error.  Since under
        # Python2 it's a little annoying to get the UTC timezone, we steal it.
        if now is None:
            now = datetime.now(instance.launch_time.tzinfo)
        age = now - instance.launch_time
        age_str = str(age)
        # strip off sub-seconds
        age_str = age_str[:age_str.find('.')]

        print((instance.id, state, group, age_str, ["%s: %s" % (k, tags[k]) for k in sorted(tags.keys())]))

def prompt(text):
    while True:
        reply = str(input(text + " (y/n) ")).lower()
        if reply[0] == 'y':
            return True
        elif reply[0] == 'n':
            return False

def ensure_started(instance):
    state = instance.state['Name']
    if state == 'running':
        return False

    if not prompt("Instance is currently %s, attempt to start it?" % state):
        print("Cannot connect to stopped instance!")
        sys.exit(1)

    client.start_instances(InstanceIds=[instance.id])
    print("Awaiting instance start...")
    awslib.await_instance(client, instance.id, None, 'running')
    return state

def restore_state(instance, old_state):
    if old_state == 'stopped':
        client.stop_instances(InstanceIds=[instance.id])
        print("Awaiting instance stop...")
        awslib.await_instance(client, instance.id, None, old_state)
    else:
        print(("Unrecognized initial state %s, cannot restore state!", old_state))

def change_security(instance, make_secure):
    secure_suffix = '-secure'

    group = instance.security_groups[0]['GroupName']
    if group.endswith(secure_suffix):
        new_group_name = group[:-len(secure_suffix)]
    else:
        new_group_name = group

    if make_secure:
        new_group_name += secure_suffix

    if new_group_name == group:
        return False

    vpc = list(ec2.vpcs.all())[0]
    new_group = vpc.security_groups.filter(GroupNames=[new_group_name])
    new_group = list(new_group)[0]

    print(('Changing instance security group to', new_group.group_name, '--', new_group))

    instance.modify_attribute(Groups=[new_group.id])
    return True

def log_into(instance):
    old_state = ensure_started(instance)
    sec_changed = change_security(instance, False)

    # If there is a private key at ~/.aws/private_key.pem, use it
    identity_args = []
    privkey_file = os.path.expanduser('~/.aws/private_key.pem')
    if os.path.isfile(privkey_file):
        print(('Using %s as identity keyfile' % privkey_file))
        identity_args = ['-i', privkey_file]

    print(('Connecting to', instance.public_ip_address))
    p = subprocess.Popen(['ssh'] + identity_args + ['ubuntu@' + instance.public_ip_address])
    p.wait()

    if sec_changed:
        change_security(instance, True)
    if old_state is not False:
        if prompt("Instance was started before connection, attempt to restore original state '%s'?" % old_state):
            restore_state(instance, old_state)

    sys.exit(p.returncode)

if len(sys.argv) == 1:
    print(('usage: %s <instance-id>' % sys.argv[0]))
    print()
    print('Current instances:')
    print_instances()
    sys.exit(0)

id = sys.argv[1]
instance = ec2.Instance(id)
log_into(instance)
