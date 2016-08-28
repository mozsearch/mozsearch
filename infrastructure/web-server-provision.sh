#!/bin/bash

set -e
set -x

sudo apt-get update
sudo apt-get install -y git

# Livegrep (Bazel is needed for Livegrep builds)
echo "deb [arch=amd64] http://storage.googleapis.com/bazel-apt stable jdk1.8" | sudo tee /etc/apt/sources.list.d/bazel.list
curl https://storage.googleapis.com/bazel-apt/doc/apt-key.pub.gpg | sudo apt-key add -
sudo apt-get update
sudo apt-get install -y bazel libssl-dev

# pygit2
sudo apt-get install -y python-virtualenv python-dev libffi-dev cmake

# Other
sudo apt-get install -y parallel realpath unzip python-pip

# Nginx
sudo apt-get install -y nginx

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install codesearch.
git clone https://github.com/livegrep/livegrep
pushd livegrep
bazel build //src/tools:codesearch
sudo install bazel-bin/src/tools/codesearch /usr/local/bin
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
    echo "usage: $0 <branch> <config-repo>"
    exit 1
fi

BRANCH=$1
CONFIG_REPO=$2

echo Branch is $BRANCH
echo Config repository is $CONFIG_REPO

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH https://github.com/bill-mccloskey/mozsearch
pushd mozsearch
git submodule init
git submodule update
popd

pushd mozsearch/tools
cargo build --release --verbose
popd

# Install files from the config repo.
git clone -b $BRANCH $CONFIG_REPO config

date
THEEND

chmod +x update.sh
