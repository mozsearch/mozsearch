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
rustup component remove clippy || true
rustup component remove rustfmt || true
rustup component remove rust-docs || true
rustup update

pushd mozsearch/tools
CARGO_INCREMENTAL=false cargo install --path . --verbose
rm -rf target
popd

# TODO: remove after next provisioning
if [ -d "$HOME/livegrep-grpc3/src" ]; then
  rm -rf "$HOME/livegrep-grpc3/src"

  LIVEGREP_VENV=$HOME/livegrep-venv
  PATH=$LIVEGREP_VENV/bin:$PATH

  git clone https://github.com/livegrep/livegrep --revision=44b2fb62ac4685ab3070f030d7130a21c2f67e31 --depth=1

  rm -rf livegrep-grpc3
  mkdir livegrep-grpc3
  pushd livegrep
  sed 's|import "src/proto/config.proto";|import "livegrep/config.proto";|' -i src/proto/livegrep.proto
  mkdir build
  python3 -m grpc_tools.protoc --python_out=build --grpc_python_out=build -I livegrep=src/proto "src/proto/config.proto" "src/proto/livegrep.proto"
  popd
  mv livegrep/build/livegrep livegrep-grpc3/livegrep
  rm -rf livegrep
fi
