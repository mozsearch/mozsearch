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

# Install Nix-provided packages
sudo -i nix profile add "$(pwd)/mozsearch#serverPackages" --accept-flake-config --print-build-logs --priority 4
# If the package was already installed we need to explicitly upgrade as well
sudo -i nix profile upgrade serverPackages --accept-flake-config --print-build-logs

# Install SpiderMonkey's shell, as indexer-update.sh does, for try imports'
# JS analysis (scripts/try-analyze-js.sh runs js-analyze.js; see
# tools/src/try_import.rs).
WGET_RETRY=(--tries=6 --waitretry=30 --retry-on-http-error=429,500,502,503,504)
JSSHELL=$(mktemp -d)
pushd "$JSSHELL"
wget -nv "${WGET_RETRY[@]}" https://firefox-ci-tc.services.mozilla.com/api/index/v1/task/gecko.v2.mozilla-central.latest.firefox.linux64-opt/artifacts/public/build/target.jsshell.zip
unzip -q target.jsshell.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd
rm -rf "$JSSHELL"
