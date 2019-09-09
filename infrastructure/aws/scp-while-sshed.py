# This is a helper to let you copy files from a machine that you've already
# ssh'ed into via `ssh.py`.  `ssh.py` handles changing the security bits for
# the instance so that ssh/scp can connect to the server.  If you try and run
# this script without ssh.py currently being connected, this script will not
# work.
#
# The general syntax is
#   python scp-while-sshed.py <instance-id> <remote-file-path> <local-target>
#
# If you wanted to copy /foo/bar/baz to the current directory
#   python scp-while-sshed.py INSTANCEID /foo/bar/baz .
#
# If you wanted to copy all log files from /fancy/path to ~/local/dir
#  python scp-while-sshed.py INSTANCEID "/fancy/path/*.log" ~/local/dir
#
# The key thing with wildcards is the standard shell expansion thing where if
# you fail to double-quote the argument, path expansion may be performed in your
# own local context.  (Sometimes it turns out okay because if the shell doesn't
# find any matches, it just passes the string through unchanged.)
#
# You CANNOT pass multiple arguments for the source, because the script is
# currently very simple and only prefixes the first argument you give it, and
# only consumes 2 path arguments.
#
# Other usage notes:
# - If you don't pass enough arguments the script yells at you and prints the
#   correct usage.
# - Unlike ssh.py, we don't show a list of all instances because you already
#   need to be connected.  (And you might be connected to multiple instance, so
#   we can't guess.)
# - If you want to copy a file TO the machine, you need to update this script or
#   fork it to make a TO version.

import boto3
from datetime import datetime
import os
import sys
import subprocess
import time

ec2 = boto3.resource('ec2')

def scp_from(instance, file_on_host, local_target):
    # If there is a private key at ~/.aws/private_key.pem, use it
    identity_args = []
    privkey_file = os.path.expanduser('~/.aws/private_key.pem')
    if os.path.isfile(privkey_file):
        print('Using %s as identity keyfile' % privkey_file)
        identity_args = ['-i', privkey_file]

    print('Connecting to', instance.public_ip_address)
    p = subprocess.Popen(['scp'] + identity_args + ['ubuntu@' + instance.public_ip_address + ':' + file_on_host, local_target])
    p.wait()

    sys.exit(p.returncode)

if len(sys.argv) < 4:
    print('usage: %s <instance-id> <remote-file-path> <local-target>' % sys.argv[0])
    sys.exit(0)

id = sys.argv[1]
instance = ec2.Instance(id)
file_on_host = sys.argv[2]
local_target = sys.argv[3]
scp_from(instance, file_on_host, local_target)
