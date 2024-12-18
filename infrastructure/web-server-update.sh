#!/usr/bin/env bash
#
# This script is run on the web server by the `update.sh` script created by the
# provisioning process.  Its purpose is to:
# 1. Download/update dependencies that change frequently and need to be
#    up-to-date.  Currently this is rust and we stay up-to-date for consistency
#    with the indexer.
# 2. Perform any necessary build steps for mozsearch for web serving.
#
# When developing, this is also a good place to:
# - Install any additional dependencies you might need.
# - Perform any new build steps your changes need.
#
# However, when it comes time to land, it's preferable to make sure that
# dependencies that don't change should just be installed once at provisioning
# time.
#

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Fix frozen PATH in .profile
# TODO: remove after next provisioning
sed 's|PATH=\(.*livegrep-venv/bin\):.*|PATH=\1:$PATH|' -i ~/.profile

# Install Nix TODO: remove after next provisioning
if ! command -v nix > /dev/null; then
    curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install linux --extra-conf "sandbox = false" --init none --no-confirm
    . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh
fi

# Install Nix-provided packages
sudo -i nix profile add "$(pwd)/mozsearch#serverPackages" --accept-flake-config --print-build-logs --priority 4
