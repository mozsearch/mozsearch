#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

USE_UID=$1
USE_GID=$2

# This all happens as root right now!

# Install core tools our Ubuntu install usually has:
apt-get update
apt-get install -y apt-utils lsb-release sudo curl wget

# Create the vagrant user with the same UID/GID as the current host user.
#
# Remove the the default "ubuntu" user, because it may conflict with UID.
userdel ubuntu
USERNAME=vagrant
useradd -u $USE_UID -o -ms /bin/bash $USERNAME
groupmod -o -g $USE_GID $USERNAME
usermod -aG sudo $USERNAME && echo "$USERNAME ALL=(ALL) NOPASSWD: ALL" > /etc/sudoers.d/$USERNAME
chmod 0440 /etc/sudoers.d/$USERNAME

# This bind point `/vagrant` is technically separate from the username.
mkdir /vagrant
chown $USERNAME:$USERNAME /vagrant
