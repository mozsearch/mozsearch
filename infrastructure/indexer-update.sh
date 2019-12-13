#!/usr/bin/env bash
#
# This script is run on the indexer by the `update.sh` script created by the
# provisioning process.  Its purpose is to:
# 1. Download/update dependencies that change frequently and need to be
#    up-to-date for indexing/analysis reasons (ex: spidermonkey for JS, rust).
# 2. Perform the build steps for mozsearch.
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

# Update Rust (make sure we have the latest version).
# We need rust nightly to use the save-analysis, and firefox requires recent
# versions of Rust.
rustup update

# Install SpiderMonkey.
rm -rf jsshell-linux-x86_64.zip js
wget -nv https://firefox-ci-tc.services.mozilla.com/api/index/v1/task/gecko.v2.mozilla-central.latest.firefox.linux64-opt/artifacts/public/build/target.jsshell.zip
mkdir js
pushd js
unzip ../target.jsshell.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd

pushd mozsearch/clang-plugin
make
popd

pushd mozsearch/tools
cargo build --release --verbose
popd
