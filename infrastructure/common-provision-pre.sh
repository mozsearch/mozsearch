#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

MOZSEARCH_REPO="${MOZSEARCH_REPO:-https://github.com/mozsearch/mozsearch}"
MOZSEARCH_BRANCH="${MOZSEARCH_BRANCH:-master}"
MOZSEARCH_CONFIG_REPO="${MOZSEARCH_CONFIG_REPO:-https://github.com/mozsearch/mozsearch-mozilla}"
MOZSEARCH_CONFIG_BRANCH="${MOZSEARCH_CONFIG_BRANCH:-master}"

# We currently try to keep the version of clang we use matching the one that
# will be used by the Firefox build process.  If you have a "mach bootstrap"ped
# system then you can see the current version locally via
# "~/.mozbuild/clang/bin/clang --version"
#
# Note that for the most recent LLVM/clang release (ex: right now v13), you
# would actually want to leave this empty.  Check out https://apt.llvm.org/ for
# the latest info in all cases.
CLANG_SUFFIX=-18
# Bumping the priority with each version upgrade lets running the provisioning
# script on an already provisioned machine do the right thing alternative-wise.
# Actually, we no longer support re-provisioning, but it's fun to increment
# numbers.
CLANG_PRIORITY=414
# The clang packages build the Ubuntu release name in; let's dynamically extract
# it since I, asuth, once forgot to update this.
UBUNTU_RELEASE=$(lsb_release -cs)

sudo apt-get update
# software-properties-common: necessary for apt-add-repository to exist
# gettext-base: necessary for `envsubst` to exist
# zip: used to create lambda zips
sudo apt-get install -y software-properties-common gettext-base rsync zip

sudo apt-add-repository -y ppa:git-core/ppa    # For latest git
sudo apt-get update
sudo apt-get install -y git
git config --global pull.ff only

# we have git, so let's check out mozsearch now so we can have our email sending
# script in case of an error.
if [ ! -d mozsearch ]; then
  mkdir mozsearch
  pushd mozsearch
  git init
  git remote add origin "$MOZSEARCH_REPO"
  git fetch origin "$MOZSEARCH_BRANCH"
  git switch --detach FETCH_HEAD
  popd
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
sudo apt-get install -y parallel unzip python3-pip python3-venv lz4 file

# We want to be able to extract stuff from json (jq) and yaml (yq) and more
# easily emit JSON from the shell (jo).
sudo apt-get install -y jq jo yq

# dos2unix is used to normalize generated files from windows
sudo apt-get install -y dos2unix

# emoji font so graphviz/pango understands emoji font metrics
sudo apt-get install -y fonts-noto-color-emoji

# graphviz for diagramming
#
# We initially started using the official graphviz project debs because 22.04
# was so far behind, but now we're sticking with the official upstream because
# they update so frequently and we are a cutting edge user of graphviz so it's
# nice to have all the fixes and enhancements ASAP.
GRAPHVIZ_DEB_BUNDLE=ubuntu_24.04_graphviz-12.0.0-debs.tar.xz
if [ ! -d $HOME/graphviz-install ]; then
  mkdir -p $HOME/graphviz-install
  pushd $HOME/graphviz-install
  curl -O https://gitlab.com/api/v4/projects/4207231/packages/generic/graphviz-releases/12.0.0/$GRAPHVIZ_DEB_BUNDLE
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

# Install gcc-12 because bazel 5.x can't build with gcc-13 because of problems
# with abseil and simply telling the livesearch bazel to use the latest bazel or
# clang just gives us different problems.
sudo apt-get install -y gcc-12 g++-12
sudo update-alternatives --install /usr/bin/gcc gcc /usr/bin/gcc-12 ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/g++ g++ /usr/bin/g++-12 ${CLANG_PRIORITY}

# Clang
wget -O - https://apt.llvm.org/llvm-snapshot.gpg.key | sudo apt-key add -
sudo apt-add-repository -y "deb https://apt.llvm.org/${UBUNTU_RELEASE}/ llvm-toolchain-${UBUNTU_RELEASE}${CLANG_SUFFIX} main"
sudo apt-get update
sudo apt-get install -y clang${CLANG_SUFFIX} libclang${CLANG_SUFFIX}-dev lld${CLANG_SUFFIX}

# Setup direct links to clang, including having clang be cc/c++
sudo update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/cc cc /usr/bin/clang${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++${CLANG_SUFFIX} ${CLANG_PRIORITY}
sudo update-alternatives --install /usr/bin/c++ c++ /usr/bin/clang${CLANG_SUFFIX} ${CLANG_PRIORITY}
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
fi
rustup install nightly
rustup default nightly
rustup uninstall stable
rustup component add rust-analyzer

# install ripgrep so we can stop experiencing grep pain / footguns
sudo apt-get install ripgrep

# Install tools for web-analyze WASM bindings.
cargo install wasm-pack
cargo install wasm-snip

# Install codesearch.
if [ ! -d livegrep ]; then
  git clone -b mozsearch-version7 https://github.com/mozsearch/livegrep --depth=1
  pushd livegrep
  $BAZEL build //src/tools:codesearch
  sudo install bazel-bin/src/tools/codesearch /usr/local/bin
  popd
  # Remove ~2G of build artifacts that we don't need anymore
  rm -rf .cache/bazel

  # Install gRPC python libs and generate the python modules to communicate with the codesearch server
  # We need to create a venv for this because Ubuntu 24.04 gets very angry if we
  # use pip to install things outside of a venv.
  LIVEGREP_VENV=$HOME/livegrep-venv
  python3 -m venv $LIVEGREP_VENV
  $LIVEGREP_VENV/bin/pip install grpcio grpcio-tools
  # also install "six" in this venv for xpidl.py for now
  $LIVEGREP_VENV/bin/pip install six

  mkdir livegrep-grpc3
  $LIVEGREP_VENV/bin/python3 -m grpc_tools.protoc --python_out=livegrep-grpc3 --grpc_python_out=livegrep-grpc3 -I livegrep/ livegrep/src/proto/config.proto
  $LIVEGREP_VENV/bin/python3 -m grpc_tools.protoc --python_out=livegrep-grpc3 --grpc_python_out=livegrep-grpc3 -I livegrep/ livegrep/src/proto/livegrep.proto
  touch livegrep-grpc3/src/__init__.py
  touch livegrep-grpc3/src/proto/__init__.py
  # Add the generated modules to the python path
  SITEDIR=$($LIVEGREP_VENV/bin/python3 -c "import site; print(site.getsitepackages()[0])")
  mkdir -p "$SITEDIR"
  echo "$PWD/livegrep-grpc3" > "$SITEDIR/livegrep.pth"
  rm -rf livegrep
fi

sudo apt-get install -y python3-boto3 python3-rich

# Install git-cinnabar.
if [ ! -d git-cinnabar ]; then
  # Need mercurial to prevent cinnabar from spewing warnings.
  sudo apt-get install -y mercurial
  # We started pinning in https://bugzilla.mozilla.org/show_bug.cgi?id=1779939
  # and it seems reasonable to stick to this for more deterministic provisioning.
  CINNABAR_REVISION=0.6.3
  git clone https://github.com/glandium/git-cinnabar -b $CINNABAR_REVISION --depth=1
  pushd git-cinnabar
    ./download.py --branch release
    # These need to be symlinks rather than `install`d binaries because cinnabar
    # uses other python code from the repo.
    for file in git-cinnabar git-cinnabar-helper git-remote-hg; do
      sudo ln -fs $(pwd)/$file /usr/local/bin/$file
    done
  popd
fi

# Install scip
SCIP_VERSION=v0.5.0
curl -L https://github.com/sourcegraph/scip/releases/download/$SCIP_VERSION/scip-linux-amd64.tar.gz | tar xzf - scip
sudo ln -fs $(pwd)/scip /usr/local/bin/scip
