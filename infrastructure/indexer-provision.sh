#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# We currently try to keep the version of clang we use matching the one that
# will be used by the Firefox build process.  If you have a "mach bootstrap"ped
# system then you can see the current version locally via
# "~/.mozbuild/clang/bin/clang --version"
CLANG_VERSION=8

sudo apt-get update
# necessary for apt-add-repository to exist
sudo apt-get install -y software-properties-common

sudo add-apt-repository -y ppa:git-core/ppa    # For latest git
sudo apt-get update

sudo apt-get install -y git
sudo apt-get install -y curl

# dos2unix is used to normalize generated files from windows
sudo apt-get install -y dos2unix

# Livegrep (Bazel is needed for Livegrep builds, OpenJDK 8 required for bazel)
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

# Clang
wget -O - http://apt.llvm.org/llvm-snapshot.gpg.key | sudo apt-key add -
sudo apt-add-repository "deb http://apt.llvm.org/bionic/ llvm-toolchain-bionic-${CLANG_VERSION} main"
sudo apt-get update
sudo apt-get install -y clang-${CLANG_VERSION} libclang-${CLANG_VERSION}-dev

# Other
sudo apt-get install -y parallel python-virtualenv python-pip

# Firefox: https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
wget -O bootstrap.py https://hg.mozilla.org/mozilla-central/raw-file/default/python/mozboot/bin/bootstrap.py && python bootstrap.py --application-choice=browser --no-interactive && rm bootstrap.py

# pygit2
sudo apt-get install -y python-dev libffi-dev cmake

# Setup direct links to clang
sudo update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config-${CLANG_VERSION} 410
sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang-${CLANG_VERSION} 410
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-${CLANG_VERSION} 410

# Install Rust. We need rust nightly to use the save-analysis
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

# Install AWS scripts.
sudo pip install boto3

# Install pygit2.
rm -rf libgit2-0.27.1
wget -nv https://github.com/libgit2/libgit2/archive/v0.27.1.tar.gz
tar xf v0.27.1.tar.gz
rm -rf v0.27.1.tar.gz
pushd libgit2-0.27.1
cmake .
make
sudo make install
popd
sudo ldconfig
sudo pip install pygit2

# Install pandoc
sudo apt-get install -y pandoc

# Install nodejs >= 8.11.3, needed for mozilla-central build
curl -sSfL https://deb.nodesource.com/setup_8.x | sudo bash
sudo apt-get install -y nodejs

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

# Update Rust (make sure we have the latest version).
# We need rust nightly to use the save-analysis, and firefox requires recent
# versions of Rust.
rustup update

# Install SpiderMonkey.
rm -rf jsshell-linux-x86_64.zip js
wget -nv https://index.taskcluster.net/v1/task/gecko.v2.mozilla-central.nightly.latest.firefox.linux64-opt/artifacts/public/build/target.jsshell.zip
mkdir js
pushd js
unzip ../target.jsshell.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH $MOZSEARCH_REPO mozsearch
pushd mozsearch
git submodule init
git submodule update
popd

pushd mozsearch/clang-plugin
make
popd

pushd mozsearch/tools
cargo build --release --verbose
popd

# Install files from the config repo.
git clone $CONFIG_REPO config
pushd config
git checkout $BRANCH || true
popd

date
THEEND

chmod +x update.sh
