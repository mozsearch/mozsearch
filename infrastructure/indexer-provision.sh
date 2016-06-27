#!/bin/bash

set -e
set -x

sudo apt-get update

sudo apt-get install -y git

# Firefox: https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
sudo apt-get install -y zip unzip mercurial g++ make autoconf2.13 yasm libgtk2.0-dev libgtk-3-dev libglib2.0-dev libdbus-1-dev libdbus-glib-1-dev libasound2-dev libcurl4-openssl-dev libiw-dev libxt-dev mesa-common-dev libgstreamer0.10-dev libgstreamer-plugins-base0.10-dev libpulse-dev m4 flex ccache libgconf2-dev clang-3.6 libclang-3.6-dev

# Livegrep
sudo apt-get install -y libgflags-dev libgit2-dev libjson0-dev libboost-system-dev libboost-filesystem-dev libsparsehash-dev cmake golang

# Other
sudo apt-get install -y parallel realpath source-highlight python-virtualenv python-dev

# pygit2
sudo apt-get install -y python-dev libffi-dev cmake

# Setup direct links to clang
sudo update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config-3.6 360
sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang-3.6 360
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-3.6 360

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install codesearch.
git clone https://github.com/livegrep/livegrep
pushd livegrep
make bin/codesearch
sudo install bin/codesearch /usr/local/bin
popd

# Install AWS scripts.
sudo pip install boto3

# Install pygit2.
wget -q https://github.com/libgit2/libgit2/archive/v0.24.0.tar.gz
tar xf v0.24.0.tar.gz
pushd libgit2-0.24.0
cmake .
make
sudo make install
popd
sudo ldconfig
sudo pip install pygit2

# Create update script.
cat > update.sh <<"THEEND"
#!/bin/bash

set -e
set -x

exec &> update-log

date

if [ $# != 2 ]
then
    echo "usage: $0 <channel> <config-repo>"
    exit 1
fi

CHANNEL=$1
CONFIG_REPO=$2

echo Channel is $CHANNEL
echo Config repository is $CONFIG_REPO

# Install SpiderMonkey.
rm -rf jsshell-linux-x86_64.zip js
wget -q https://index.taskcluster.net/v1/task/gecko.v2.mozilla-central.nightly.latest.firefox.linux64-opt/artifacts/public/build/jsshell-linux-x86_64.zip
mkdir js
pushd js
unzip ../jsshell-linux-x86_64.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd

BRANCH=master
if [ $CHANNEL != release ]
then
    BRANCH=$CHANNEL
fi

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH https://github.com/bill-mccloskey/mozsearch
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
git clone -b $BRANCH $CONFIG_REPO config

date
THEEND

chmod +x update.sh
