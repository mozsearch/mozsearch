#!/bin/bash

set -e
set -x

sudo apt-get update

sudo apt-get install -y git
sudo apt-get install -y curl
sudo apt-get install -y software-properties-common

# Livegrep (Bazel is needed for Livegrep builds, OpenJDK 8 required for bazel)
sudo apt-get install -y openjdk-8-jdk
echo "deb [arch=amd64] http://storage.googleapis.com/bazel-apt stable jdk1.8" | sudo tee /etc/apt/sources.list.d/bazel.list
curl https://storage.googleapis.com/bazel-apt/doc/apt-key.pub.gpg | sudo apt-key add -
sudo apt-get update
sudo apt-get install -y bazel libssl-dev

# Clang
wget -O - http://apt.llvm.org/llvm-snapshot.gpg.key | sudo apt-key add -
sudo apt-add-repository "deb http://apt.llvm.org/xenial/ llvm-toolchain-xenial-4.0 main"
sudo apt-get update
sudo apt-get install -y clang-4.0 clang-4.0-dev

# Firefox: https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
sudo apt-get install -y zip unzip mercurial g++ make autoconf2.13 yasm libgtk2.0-dev libgtk-3-dev libglib2.0-dev libdbus-1-dev libdbus-glib-1-dev libasound2-dev libcurl4-openssl-dev libiw-dev libxt-dev mesa-common-dev libgstreamer0.10-dev libgstreamer-plugins-base0.10-dev libpulse-dev m4 flex libx11-xcb-dev ccache libgconf2-dev

# Other
sudo apt-get install -y parallel realpath python-virtualenv python-pip

# pygit2
sudo apt-get install -y python-dev libffi-dev cmake

# Setup direct links to clang
sudo update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config-4.0 400
sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang-4.0 400
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-4.0 400

# Install Rust. We need rust nightly to use the save-analysis
curl -sSf https://static.rust-lang.org/rustup.sh | sh -s -- --channel=nightly

# Install codesearch.
rm -rf livegrep
git clone -b mozsearch-version https://github.com/mozsearch/livegrep
pushd livegrep
# The last two options turn off the bazel sandbox, which doesn't work
# inside an LDX container.
bazel build //src/tools:codesearch --incompatible_disallow_set_constructor=false --spawn_strategy=standalone --genrule_strategy=standalone
sudo install bazel-bin/src/tools/codesearch /usr/local/bin
popd

# Install AWS scripts.
sudo pip install boto3

# Install pygit2.
rm -rf libgit2-0.26.0
wget -q https://github.com/libgit2/libgit2/archive/v0.26.0.tar.gz
tar xf v0.26.0.tar.gz
rm -rf v0.26.0.tar.gz
pushd libgit2-0.26.0
cmake .
make
sudo make install
popd
sudo ldconfig
sudo pip install pygit2

# Install pandoc
sudo apt-get install -y pandoc

# Create update script.
cat > update.sh <<"THEEND"
#!/bin/bash

set -e
set -x

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

# Re-install Rust (make sure we have the latest version).
# We need rust nightly to use the save-analysis, and firefox requires recent
# versions of Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh -s -- --channel=nightly

# Install SpiderMonkey.
rm -rf jsshell-linux-x86_64.zip js
wget -q https://index.taskcluster.net/v1/task/gecko.v2.mozilla-central.nightly.latest.firefox.linux64-opt/artifacts/public/build/target.jsshell.zip
mkdir js
pushd js
unzip ../target.jsshell.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH $MOZSEARCH_REPO
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
