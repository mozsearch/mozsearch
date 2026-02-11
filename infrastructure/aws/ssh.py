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

def print_instances(select, multiple, patterns):
    now = None

    id_map = {}
    id_list = []
    current_index = 1

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

        tag_list = ['%s: %s' % (k, tags[k]) for k in sorted(tags.keys())]
        line = f"{instance.id} {state} {group} {age_str} {tag_list}"

        if patterns is not None:
            matches = True
            for p in patterns:
                if p not in line:
                    matches = False
                    break
            if not matches:
                continue

        if multiple:
            id_list.append([instance.id, line])
            continue

        if select:
            print(' {}) '.format(current_index), end='')
            id_map[str(current_index)] = instance.id
            current_index += 1

        print(line)

    if select:
        print()
        while True:
            index = input('index: ')
            if index in id_map:
                return id_map[index]

    if multiple:
        return id_list

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
    print("Instance switched to running state, waiting 20s for SSH server to start...")
    time.sleep(20)
    return state

def restore_state(instance, old_state):
    if old_state == 'stopped':
        client.stop_instances(InstanceIds=[instance.id])
        print("Awaiting instance stop...")
        awslib.await_instance(client, instance.id, None, old_state)
    else:
        print("Unrecognized initial state %s, cannot restore state!" % old_state)

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

    print('Changing instance security group to', new_group.group_name, '--', new_group)

    instance.modify_attribute(Groups=[new_group.id])
    return True

def log_into(instance, remote_cmd=[]):
    old_state = ensure_started(instance)
    sec_changed = change_security(instance, False)

    # If there is a private key at ~/.aws/private_key.pem, use it
    identity_args = []
    privkey_file = os.path.expanduser('~/.aws/private_key.pem')
    if os.path.isfile(privkey_file):
        print('Using %s as identity keyfile' % privkey_file)
        identity_args = ['-i', privkey_file]

    # Disable host key checking and the pollution of the user's own known host keys.
    # The rationale is:
    # - These server keys are basically ephemeral.
    # - Before this change, we already didn't bother verifying that the ssh keys
    #   were as expected.
    # Good next steps would be:
    # - Use the AWS API to find out the server's ssh key and create a transient
    #   known hosts file that's pre-populated and that we can use.
    hostkey_args = ["-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no"]

    print('Connecting to', instance.public_ip_address)
    p = subprocess.Popen(['ssh'] + hostkey_args + identity_args + ['ubuntu@' + instance.public_ip_address] + remote_cmd)
    p.wait()

    if sec_changed:
        change_security(instance, True)
    if old_state is not False:
        if prompt("Instance was started before connection, attempt to restore original state '%s'?" % old_state):
            restore_state(instance, old_state)

    if len(remote_cmd):
        print("Return code =", p.returncode)
        return
    sys.exit(p.returncode)

if len(sys.argv) == 1:
    print('usage: %s (<instance-id>|patterns) [command args...]' % sys.argv[0])
    print()
    print('  instance-id: i-* style ID for the instance')
    print('  patterns: A comma-separated list of patterns, to select the')
    print('            instances by substring-match filter.')
    print('            If no command is provided, prompt for selecting an')
    print('            instance to login.')
    print('            If a command is provided, run the command in the')
    print('            all matching instances.')
    print('  command: A command line to run on the specified instances')
    print()
    print('Current instances:')
    print_instances(select=False, multiple=False, patterns=None)
    sys.exit(0)

id = sys.argv[1]

if len(sys.argv) >= 3:
    patterns = sys.argv[1].split(',')
    remote_cmd = sys.argv[2:]

    ids = print_instances(select=False, multiple=True, patterns=patterns)

    for [id, line] in ids:
        instance = ec2.Instance(id)
        print("#", line)
        log_into(instance, remote_cmd)
    sys.exit(0)

if not id.startswith('i-'):
    patterns = sys.argv[1].split(',')

    print('Current instances:')
    id = print_instances(select=True, multiple=False, patterns=patterns)

instance = ec2.Instance(id)
log_into(instance)
