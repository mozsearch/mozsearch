#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

sudo apt-get update
sudo apt-get install -y git

# Livegrep (Bazel is needed for Livegrep builds)
sudo apt-get install -y unzip openjdk-8-jdk libssl-dev
# Install Bazel 0.7.0
rm -rf bazel
mkdir bazel
pushd bazel
# Note that bazel unzips itself so we can't just pipe it to sudo bash.
curl -sSfL -O https://github.com/bazelbuild/bazel/releases/download/0.7.0/bazel-0.7.0-without-jdk-installer-linux-x86_64.sh
chmod +x bazel-0.7.0-without-jdk-installer-linux-x86_64.sh
sudo ./bazel-0.7.0-without-jdk-installer-linux-x86_64.sh
popd

# pygit2
sudo apt-get install -y python-virtualenv python-dev libffi-dev cmake

# Other
sudo apt-get install -y parallel realpath unzip python-pip

# Nginx
sudo apt-get install -y nginx

# Install pkg-config (needed for Rust's OpenSSL wrappers)
sudo apt-get install pkg-config

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh -s -- --channel=nightly

# Install codesearch.
rm -rf livegrep
git clone -b mozsearch-version2 https://github.com/mozsearch/livegrep
pushd livegrep
bazel build //src/tools:codesearch --incompatible_disallow_set_constructor=false
sudo install bazel-bin/src/tools/codesearch /usr/local/bin
popd

# Install gRPC python libs and generate the python modules to communicate with the codesearch server
sudo pip install grpcio grpcio-tools
rm -rf livegrep-grpc
mkdir livegrep-grpc
python -m grpc_tools.protoc --python_out=livegrep-grpc --grpc_python_out=livegrep-grpc -I livegrep/src/proto livegrep/src/proto/livegrep.proto
# Add the generated modules to the python path
SITEDIR=$(python -m site --user-site)
mkdir -p "$SITEDIR"
echo "$PWD/livegrep-grpc" > "$SITEDIR/livegrep.pth"

# Install AWS scripts.
sudo pip install boto3

# Install pygit2.
rm -rf libgit2-0.27.1
wget -q https://github.com/libgit2/libgit2/archive/v0.27.1.tar.gz
tar xf v0.27.1.tar.gz
rm -rf v0.27.1.tar.gz
pushd libgit2-0.27.1
cmake .
make
sudo make install
popd
sudo ldconfig
sudo pip install pygit2

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

# Re-install Rust (make sure we have the latest version).
curl -sSf https://static.rust-lang.org/rustup.sh | sh -s -- --channel=nightly

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
