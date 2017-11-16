#!/bin/bash

set -e
set -x

sudo apt-get update
sudo apt-get install -y git

# Livegrep (Bazel is needed for Livegrep builds)
sudo apt-get install -y openjdk-8-jdk
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

# Install pkg-config (needed for Rust's OpenSSL wrappers)
sudo apt-get install pkg-config

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install codesearch.
rm -rf livegrep
git clone -b mozsearch-version https://github.com/mozsearch/livegrep
pushd livegrep
bazel build //src/tools:codesearch --incompatible_disallow_set_constructor=false
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
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH $MOZSEARCH_REPO
pushd mozsearch
git submodule init
git submodule update
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
