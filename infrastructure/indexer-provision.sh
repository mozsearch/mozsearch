#!/bin/bash

set -e
set -x

sudo apt-get update

sudo apt-get install -y git

# Clang
sudo apt-get install -y clang-3.8 clang-3.8-dev

# Firefox: https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
sudo apt-get install -y zip unzip mercurial g++ make autoconf2.13 yasm libgtk2.0-dev libgtk-3-dev libglib2.0-dev libdbus-1-dev libdbus-glib-1-dev libasound2-dev libcurl4-openssl-dev libiw-dev libxt-dev mesa-common-dev libgstreamer0.10-dev libgstreamer-plugins-base0.10-dev libpulse-dev m4 flex libx11-xcb-dev ccache libgconf2-dev

# Livegrep (Bazel is needed for Livegrep builds)
echo "deb [arch=amd64] http://storage.googleapis.com/bazel-apt stable jdk1.8" | sudo tee /etc/apt/sources.list.d/bazel.list
curl https://storage.googleapis.com/bazel-apt/doc/apt-key.pub.gpg | sudo apt-key add -
sudo apt-get update
sudo apt-get install -y bazel libssl-dev

# Other
sudo apt-get install -y parallel realpath python-virtualenv python-pip

# pygit2
sudo apt-get install -y python-dev libffi-dev cmake

# Setup direct links to clang
sudo update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config-3.8 380
sudo update-alternatives --install /usr/bin/clang clang /usr/bin/clang-3.8 380
sudo update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-3.8 380

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install codesearch.
git clone https://github.com/livegrep/livegrep
pushd livegrep
git reset --hard 48a06ed14127f37e2537a14be86713ae538cebb5
# The last two options turn off the bazel sandbox, which doesn't work
# inside an LDX container.
bazel build //src/tools:codesearch --spawn_strategy=standalone --genrule_strategy=standalone
sudo install bazel-bin/src/tools/codesearch /usr/local/bin
popd

# Install AWS scripts.
sudo pip install boto3

# Install pygit2.
wget -q https://github.com/libgit2/libgit2/archive/v0.25.0.tar.gz
tar xf v0.25.0.tar.gz
pushd libgit2-0.25.0
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

if [ $# != 2 ]
then
    echo "usage: $0 <branch> <config-repo>"
    exit 1
fi

BRANCH=$1
CONFIG_REPO=$2

echo Branch is $BRANCH
echo Config repository is $CONFIG_REPO

# Re-install Rust (make sure we have the latest version).
# Building Firefox requires a recent version of Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

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
