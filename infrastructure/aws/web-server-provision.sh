#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# We want the NVME tools, that's how EBS gets mounted now on "nitro" instances.
sudo apt-get install -y nvme-cli

date
