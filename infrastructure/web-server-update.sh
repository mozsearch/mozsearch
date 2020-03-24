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

# See comments in indexer-update.sh
rustup component remove clippy
rustup component remove rustfmt
rustup component remove rust-docs
rustup update

pushd mozsearch/tools
cargo build --release --verbose
popd
