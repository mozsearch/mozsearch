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
#
# Before we do the update, we remove some components that we don't need and
# that are sometimes missing. If they are missing, `rustup update` will try
# to use a previous nightly instead that does have the components, which means
# we end up with a slightly older rustc. Using rustc from a few days ago is
# usually fine, but in cases where we hit ICEs that have been fixed upstream,
# we want the very latest rustc to get the fix. Removing these components also
# reduces download time during `rustup update`.
#
# Note that these commands are not idempotent, so we need to `|| true` for cases
# where they've already been removed by a prior invocation of this script.
# (Originally this script would only ever be run on the indexers and web-servers
# at most once because the script would not be run during provisioning and each
# VM's root partition would be discarded after running.  Now we run this script
# as part of provisioning for side-effects.)
rustup component remove clippy || true
rustup component remove rustfmt || true
rustup component remove rust-docs || true
rustup update

# Install SpiderMonkey.
rm -rf target.jsshell.zip js
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
