#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

cat > cloudwatch.cfg <<"THEEND"
[general]
state_file = /var/awslogs/state/agent-state

[/home/ubuntu/index-log]
file = /home/ubuntu/index-log
log_group_name = /home/ubuntu/index-log
log_stream_name = {instance_id}
THEEND

date

wget -q https://s3.amazonaws.com/aws-cloudwatch/downloads/latest/awslogs-agent-setup.py
chmod +x awslogs-agent-setup.py
sudo ./awslogs-agent-setup.py -n -r us-west-2 -c ./cloudwatch.cfg
