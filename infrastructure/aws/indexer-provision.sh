#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# We want the NVME tools, that's how EBS gets mounted now on "nitro" instances.
sudo apt-get install -y nvme-cli

cat > cloudwatch.cfg <<"THEEND"
[general]
state_file = /var/awslogs/state/agent-state

[/home/ubuntu/index-log]
file = /home/ubuntu/index-log
log_group_name = /home/ubuntu/index-log
log_stream_name = {instance_id}
THEEND

date

wget -nv https://s3.amazonaws.com/aws-cloudwatch/downloads/latest/awslogs-agent-setup.py
chmod +x awslogs-agent-setup.py
# Currently this claims to only work with Python 2.6 - 3.5, so we use python2
# which will use Python 2.7.
sudo python2 ./awslogs-agent-setup.py -n -r us-west-2 -c ./cloudwatch.cfg
