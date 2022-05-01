#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

sudo add-apt-repository ppa:git-core/ppa    # For latest git
sudo apt-get update
sudo apt-get install -y git
git config --global pull.ff only

# we have git, so let's check out mozsearch now so we can have our email sending
# script in case of an error.  (For the VM the dir may already exist thanks to
# us having already provisioned the indexer.)
if [ ! -d mozsearch ]; then
  git clone -b master https://github.com/mozsearch/mozsearch mozsearch
fi

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
sudo apt-get install -y gdb python3-dbg

# Other
sudo apt-get install -y parallel unzip python3-pip

# and we want to be able to extract stuff from json and yaml
sudo apt-get install -y jq
sudo pip3 install yq

# Livegrep (Bazel is needed for Livegrep builds, OpenJDK 8 required for bazel)
sudo apt-get install -y unzip openjdk-8-jdk libssl-dev
# Install Bazel 1.1.0
if [ ! -d bazel ]; then
  mkdir bazel
  pushd bazel
    # Note that bazel unzips itself so we can't just pipe it to sudo bash.
    curl -sSfL -O https://github.com/bazelbuild/bazel/releases/download/1.1.0/bazel-1.1.0-installer-linux-x86_64.sh
    chmod +x bazel-1.1.0-installer-linux-x86_64.sh
    sudo ./bazel-1.1.0-installer-linux-x86_64.sh
  popd
fi

# Nginx
sudo apt-get install -y nginx

# Install pkg-config (needed for Rust's OpenSSL wrappers)
sudo apt-get install -y pkg-config

# rust gRPC via tonic/tonic-build and prost-build needs cmake
sudo apt-get install -y cmake

# Install Rust. We need rust nightly to build rls-data.
if [ ! -d $HOME/.cargo ]; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
  source $HOME/.cargo/env
  rustup install nightly
  rustup default nightly
  rustup uninstall stable
fi

# Install codesearch.
if [ ! -d livegrep ]; then
  git clone -b mozsearch-version5 https://github.com/mozsearch/livegrep
  pushd livegrep
    bazel build //src/tools:codesearch
    sudo install bazel-bin/src/tools/codesearch /usr/local/bin
  popd
  # Remove ~2G of build artifacts that we don't need anymore
  rm -rf .cache/bazel
fi

# Install gRPC python libs and generate the python modules to communicate with the codesearch server
sudo pip3 install grpcio grpcio-tools
mkdir livegrep-grpc3
python3 -m grpc_tools.protoc --python_out=livegrep-grpc3 --grpc_python_out=livegrep-grpc3 -I livegrep/ livegrep/src/proto/config.proto
python3 -m grpc_tools.protoc --python_out=livegrep-grpc3 --grpc_python_out=livegrep-grpc3 -I livegrep/ livegrep/src/proto/livegrep.proto
touch livegrep-grpc3/src/__init__.py
touch livegrep-grpc3/src/proto/__init__.py
# Add the generated modules to the python path
SITEDIR=$(python3 -m site --user-site)
mkdir -p "$SITEDIR"
echo "$PWD/livegrep-grpc3" > "$SITEDIR/livegrep.pth"

# graphviz for diagramming
sudo apt-get install -y graphviz

# Install AWS scripts and command-line tool.
#
# Note that we don't run the web server with an IAM role so the `aws` tool won't
# be able to do anything unless you provide it with credentials.  Indexers,
# however, do have a role, so if you want to noodle around with something, use
# an indexer.
sudo pip3 install boto3 awscli rich

# Install git-cinnabar.
if [ ! -d git-cinnabar ]; then
  # Need mercurial to prevent cinnabar from spewing warnings, and cinnabar requires python2.7
  sudo apt-get install -y mercurial python2.7
  CINNABAR_REVISION=release
  git clone https://github.com/glandium/git-cinnabar
  pushd git-cinnabar
    git checkout $CINNABAR_REVISION
    ./git-cinnabar download
    # These need to be symlinks rather than `install`d binaries because cinnabar
    # uses other python code from the repo.
    for file in git-cinnabar git-cinnabar-helper git-remote-hg; do
      sudo ln -fs $(pwd)/$file /usr/local/bin/$file
    done
  popd
fi

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
rm -rf config
git clone $CONFIG_REPO config
pushd config
git checkout $BRANCH -- || true
popd

date

# Let mozsearch tell us what commonly changing dependencies to install plus
# perform any build steps.
mozsearch/infrastructure/web-server-update.sh

date
THEEND

chmod +x update.sh

# Run the update script for a side effect of downloading the crates.io
# dependencies ahead of time since we're seeing intermittent network problems
# downloading crates in https://bugzilla.mozilla.org/show_bug.cgi?id=1720037.
#
# Note that because the update script fully deletes the mozsearch directory,
# this really is just:
# - Validating the image can compile and use rust and clang correctly.
# - Caching some crates in `~/.cargo`.
./update.sh master https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla
mv update-log provision-update-log-1

# Run this a second time to make sure the script is actually idempotent, so we
# don't have any surprises when the update script gets run when the VM spins up.
./update.sh master https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla
mv update-log provision-update-log-2
