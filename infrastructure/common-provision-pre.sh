#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# We currently try to keep the version of clang we use matching the one that
# will be used by the Firefox build process.  If you have a "mach bootstrap"ped
# system then you can see the current version locally via
# "~/.mozbuild/clang/bin/clang --version"
#
# Note that for the most recent LLVM/clang release (ex: right now v13), you
# would actually want to leave this empty.  Check out https://apt.llvm.org/ for
# the latest info in all cases.
CLANG_SUFFIX=-17
# Bumping the priority with each version upgrade lets running the provisioning
# script on an already provisioned machine do the right thing alternative-wise.
# Actually, we no longer support re-provisioning, but it's fun to increment
# numbers.
CLANG_PRIORITY=413
# The clang packages build the Ubuntu release name in; let's dynamically extract
# it since I, asuth, once forgot to update this.
UBUNTU_RELEASE=$(lsb_release -cs)

sudo apt-get update
# software-properties-common: necessary for apt-add-repository to exist
# gettext-base: necessary for `envsubst` to exist
sudo apt-get install -y software-properties-common gettext-base rsync

sudo add-apt-repository -y ppa:git-core/ppa    # For latest git
sudo apt-get update
sudo apt-get install -y git
git config --global pull.ff only

# we have git, so let's check out mozsearch now so we can have our email sending
# script in case of an error.
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
sudo apt-get install -y parallel unzip python3-pip python3-venv lz4

# We want to be able to extract stuff from json (jq) and yaml (yq) and more
# easily emit JSON from the shell (jo).
sudo apt-get install -y jq jo
sudo pip3 install yq

# dos2unix is used to normalize generated files from windows
sudo apt-get install -y dos2unix

# emoji font so graphviz/pango understands emoji font metrics
sudo apt-get install -y fonts-noto-color-emoji

# graphviz for diagramming
#
# The most recent release version is 9.0 but the version available in 22.04 is
# 2.x, so we donwload and use the official packages provided by the graphviz
# project on gitlab.
GRAPHVIZ_DEB_BUNDLE=ubuntu_22.04_graphviz-9.0.0-debs.tar.xz
if [ ! -d $HOME/graphviz-install ]; then
  mkdir -p $HOME/graphviz-install
  pushd $HOME/graphviz-install
  curl -O https://gitlab.com/api/v4/projects/4207231/packages/generic/graphviz-releases/9.0.0/$GRAPHVIZ_DEB_BUNDLE
  tar xvf $GRAPHVIZ_DEB_BUNDLE
  # using constrained wildcards here to not care too much about these versions
  sudo apt-get install -y ./graphviz_*_amd64.deb ./libgraphviz4_*_amd64.deb ./libgraphviz-dev_*_amd64.deb
  popd
fi

# Prior livegrep deps, now rust wants libssl-dev still
sudo apt-get install -y unzip libssl-dev

# Install Bazelisk to install bazel as needed.  bazezlisk is like nvm.
if [ ! -d bazelisk ]; then
  mkdir bazelisk
  pushd bazelisk
    curl -sSfL -O https://github.com/bazelbuild/bazelisk/releases/download/v1.11.0/bazelisk-linux-amd64
    chmod +x bazelisk-linux-amd64
  popd
fi
BAZEL=~/bazelisk/bazelisk-linux-amd64

# Clang
wget -O - https://apt.llvm.org/llvm-snapshot.gpg.key | sudo apt-key add -
sudo apt-add-repository "deb https://apt.llvm.org/${UBUNTU_RELEASE}/ llvm-toolchain-${UBUNTU_RELEASE}${CLANG_SUFFIX} main"
sudo apt-get update
sudo apt-get install -y clang${CLANG_SUFFIX} libclang${CLANG_SUFFIX}-dev lld${CLANG_SUFFIX}

# Setup direct links to clang
sudo update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/llvm-symbolizer llvm-symbolizer /usr/bin/llvm-symbolizer${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/lld lld /usr/bin/lld${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/ld.lld ld.lld /usr/bin/ld.lld${CLANG_SUFFIX} ${CLANG_PRIORITY}

# Install pkg-config (needed for Rust's OpenSSL wrappers)
# Install vmtouch for caching files into memory
sudo apt-get install -y pkg-config vmtouch

# rust gRPC via tonic/tonic-build and prost-build needs protoc (and cmake?)
sudo apt-get install -y cmake protobuf-compiler

# Install Rust. We need rust nightly to use the save-analysis
if [ ! -d $HOME/.cargo ]; then
  curl https://sh.rustup.rs -sSf | sh -s -- -y
  source $HOME/.cargo/env
  rustup install nightly
  rustup default nightly
  rustup uninstall stable
fi

# install ripgrep so we can stop experiencing grep pain / footguns
cargo install ripgrep

# Install tools for web-analyze WASM bindings.
cargo install wasm-pack
cargo install wasm-snip

# Install codesearch.
if [ ! -d livegrep ]; then
  git clone -b mozsearch-version6 https://github.com/mozsearch/livegrep
  pushd livegrep
    $BAZEL build //src/tools:codesearch
    sudo install bazel-bin/src/tools/codesearch /usr/local/bin
  popd
  # Remove ~2G of build artifacts that we don't need anymore
  rm -rf .cache/bazel

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
fi

# Install AWS scripts and command-line tool.
#
# awscli can get credentials via `Ec2InstanceMetadata` which will give it the
# authorities of the role assigned to the image it's running in.  Look for the
# `IamInstanceProfile` definition in `trigger_indexer.py` and similar.
sudo pip3 install boto3 awscli rich

# Install git-cinnabar.
if [ ! -d git-cinnabar ]; then
  # Need mercurial to prevent cinnabar from spewing warnings.
  # python2.7 is also currently needed, but was installed above.
  sudo apt-get install -y mercurial
  # We started pinning in https://bugzilla.mozilla.org/show_bug.cgi?id=1779939
  # and it seems reasonable to stick to this for more deterministic provisioning.
  CINNABAR_REVISION=0.6.3
  git clone https://github.com/glandium/git-cinnabar
  pushd git-cinnabar
    git checkout $CINNABAR_REVISION
    ./download.py
    # These need to be symlinks rather than `install`d binaries because cinnabar
    # uses other python code from the repo.
    for file in git-cinnabar git-cinnabar-helper git-remote-hg; do
      sudo ln -fs $(pwd)/$file /usr/local/bin/$file
    done
  popd
fi

# Install scip
SCIP_VERSION=v0.3.3
curl -L https://github.com/sourcegraph/scip/releases/download/$SCIP_VERSION/scip-linux-amd64.tar.gz | tar xzf - scip
sudo ln -fs $(pwd)/scip /usr/local/bin/scip

# Install rust-analyzer
RUST_ANALYZER_VERSION=nightly
rm -rf rust-analyzer rust-analyzer-linux-x64.vsix
wget https://github.com/rust-lang/rust-analyzer/releases/download/$RUST_ANALYZER_VERSION/rust-analyzer-linux-x64.vsix
unzip -o -d rust-analyzer rust-analyzer-linux-x64.vsix
sudo ln -fs $(pwd)/rust-analyzer/extension/server/rust-analyzer /usr/local/bin/rust-analyzer
