#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

MOZSEARCH_REPO="${MOZSEARCH_REPO:-https://github.com/mozsearch/mozsearch}"
MOZSEARCH_BRANCH="${MOZSEARCH_BRANCH:-master}"
MOZSEARCH_CONFIG_REPO="${MOZSEARCH_CONFIG_REPO:-https://github.com/mozsearch/mozsearch-mozilla}"
MOZSEARCH_CONFIG_BRANCH="${MOZSEARCH_CONFIG_BRANCH:-master}"

# Install zlib.h (needed for NSS build)
sudo apt-get install -y zlib1g-dev

# Building LLVM likes to have ninja; pernosco also can use it if we ever index that.
sudo apt-get install -y ninja-build

# cargo-insta makes it possible to use the UI documented at
# https://insta.rs/docs/cli/ to review changes to "check" scripts.  For the test
# repo, this is used by `make review-test-repo`.  It's not expected that this
# will actually be necessary on the production indexer and so this isn't part of
# the update process.
cargo install cargo-insta

# Install node.js for scip-typescript; github lists v18 and v20 as supported;
# we are sticking with v18 for now because currently all the invocations
# hardcode v18 as well; that will need to be addressed.
sudo apt install -y npm

# Install scip-typescript under node.js v18
sudo npm install -g @sourcegraph/scip-typescript

# Install scip-python under node.js v18 as well
#npm install -g @sourcegraph/scip-python
# To get my fix https://github.com/sourcegraph/scip-python/pull/150
sudo npm install -g @asutherland/scip-python

# Install a JDK and Coursier.
# v21 is currently the most recent available version of Ubuntu 24.04 (and v19 was
# removed).
sudo apt install -y openjdk-21-jdk
curl -fL "https://github.com/coursier/launchers/raw/master/cs-x86_64-pc-linux.gz" | gzip -d > cs
chmod +x cs
./cs setup --yes
# Coursier adds itself to the path from ~/.profile, but add it now too
PATH="$PATH:$HOME/.local/share/coursier/bin"

# Install scip-java
cs install --contrib scip-java
rm -rf ~/.cache/coursier

# Create update script.
cat > update.sh <<"THEEND"
#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

exec > >(tee -a update-log) 2>&1

date

if [ $# != 4 ]
then
    echo "usage: $0 <mozsearch-repo> <mozsearch-rev> <config-repo> <config-rev>"
    exit 1
fi

MOZSEARCH_REPO=$1
MOZSEARCH_REV=$2
CONFIG_REPO=$3
CONFIG_REV=$4

echo Mozsearch repository is $MOZSEARCH_REPO rev $MOZSEARCH_REV
echo Config repository is $CONFIG_REPO rev $CONFIG_REV

# Install mozsearch.
rm -rf mozsearch
mkdir mozsearch
pushd mozsearch
git init
git remote add origin "$MOZSEARCH_REPO"
git fetch origin "$MOZSEARCH_REV"
git switch --detach FETCH_HEAD
git submodule init
git submodule update
popd

# Install files from the config repo.
rm -rf config
git clone -b $CONFIG_REV $CONFIG_REPO config --depth=1

date

# Let mozsearch tell us what commonly changing dependencies to install plus
# perform any build steps.
mozsearch/infrastructure/indexer-update.sh

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
./update.sh "$MOZSEARCH_REPO" "$MOZSEARCH_BRANCH" "$MOZSEARCH_CONFIG_REPO" "$MOZSEARCH_CONFIG_BRANCH"
mv update-log provision-update-log-1

# Run this a second time to make sure the script is actually idempotent, so we
# don't have any surprises when the update script gets run when the VM spins up.
./update.sh "$MOZSEARCH_REPO" "$MOZSEARCH_BRANCH" "$MOZSEARCH_CONFIG_REPO" "$MOZSEARCH_CONFIG_BRANCH"
mv update-log provision-update-log-2
