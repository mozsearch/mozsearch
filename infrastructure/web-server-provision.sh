#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

sudo add-apt-repository ppa:git-core/ppa    # For latest git
sudo apt-get update

# the base image we're building against is inherently not up-to-date (new base
# images are released only monthly), so let's be consistently up-to-date.
sudo DEBIAN_FRONTEND=noninteractive \
  apt-get \
  -o Dpkg::Options::=--force-confold \
  -o Dpkg::Options::=--force-confdef \
  -y --allow-downgrades --allow-remove-essential --allow-change-held-packages \
  dist-upgrade

# unattended upgrades pose a problem for debugging running processes because we
# end up running version N but have debug symbols for N+1 and that doesn't work.
sudo apt-get remove -y unattended-upgrades
# and we want to be able to debug python
sudo apt-get install -y gdb
sudo apt-get install -y python-dbg

# and we want to be able to extract stuff from json
sudo apt-get install -y jq

sudo apt-get install -y git

# Livegrep (Bazel is needed for Livegrep builds)
sudo apt-get install -y unzip openjdk-8-jdk libssl-dev
# Install Bazel 0.16.1
rm -rf bazel
mkdir bazel
pushd bazel
# Note that bazel unzips itself so we can't just pipe it to sudo bash.
curl -sSfL -O https://github.com/bazelbuild/bazel/releases/download/0.22.0/bazel-0.22.0-installer-linux-x86_64.sh
chmod +x bazel-0.22.0-installer-linux-x86_64.sh
sudo ./bazel-0.22.0-installer-linux-x86_64.sh
popd

# Other
sudo apt-get install -y parallel unzip python-pip python3-pip

# Nginx
sudo apt-get install -y nginx

# Install pkg-config (needed for Rust's OpenSSL wrappers)
sudo apt-get install pkg-config

# Install Rust. We need rust nightly to build rls-data.
curl https://sh.rustup.rs -sSf | sh -s -- -y
source $HOME/.cargo/env
rustup install nightly
rustup default nightly
rustup uninstall stable

# Install codesearch.
rm -rf livegrep
git clone -b mozsearch-version4 https://github.com/mozsearch/livegrep
pushd livegrep
bazel build //src/tools:codesearch
sudo install bazel-bin/src/tools/codesearch /usr/local/bin
popd
# Remove ~2G of build artifacts that we don't need anymore
rm -rf .cache/bazel

# Install gRPC python libs and generate the python modules to communicate with the codesearch server
sudo pip install grpcio grpcio-tools
rm -rf livegrep-grpc
mkdir livegrep-grpc
python -m grpc_tools.protoc --python_out=livegrep-grpc --grpc_python_out=livegrep-grpc -I livegrep/ livegrep/src/proto/config.proto
python -m grpc_tools.protoc --python_out=livegrep-grpc --grpc_python_out=livegrep-grpc -I livegrep/ livegrep/src/proto/livegrep.proto
touch livegrep-grpc/src/__init__.py
touch livegrep-grpc/src/proto/__init__.py
# Add the generated modules to the python path
SITEDIR=$(python -m site --user-site)
mkdir -p "$SITEDIR"
echo "$PWD/livegrep-grpc" > "$SITEDIR/livegrep.pth"

# Install AWS scripts.
sudo pip install boto3
sudo pip3 install boto3

# Create update script.
cat > update.sh <<"THEEND"
#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

exec &> update-log

date

if [ $# != 3 ]
then
    echo "usage: $0 <branch> <mozsearch-repo> <config-repo>"
    exit 1
fi

BRANCH=$1
MOZSEARCH_REPO=$2
CONFIG_REPO=$3

echo Branch is $BRANCH
echo Mozsearch repository is $MOZSEARCH_REPO
echo Config repository is $CONFIG_REPO

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH $MOZSEARCH_REPO mozsearch
pushd mozsearch
git submodule init
git submodule update
popd

# Install files from the config repo.
git clone $CONFIG_REPO config
pushd config
git checkout $BRANCH || true
popd

date

# Let mozsearch tell us what commonly changing dependencies to install plus
# perform any build steps.
mozsearch/infrastructure/web-server-update.sh

date
THEEND

chmod +x update.sh
