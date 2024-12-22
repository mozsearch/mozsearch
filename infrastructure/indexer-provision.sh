#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

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

# To help install node.js and similar, we install mise, a rust-based "asdf"
# alternative, which if you don't know what "asdf" is, but know what "nvm" is,
# it's basically a super-nvm for multiple languages, etc.  We use the install
# method documented at https://mise.jdx.dev/getting-started.html#cargo but there
# are a bunch of other options.
#
# The core rationale here is that I've locally been using "nvm" for node.js
# purposes for a while now and it's been a much better experience than trying to
# use debian/ubuntu distro-provided versions of node, and in particular can be
# invaluable when trying to just get things to work when packages are involved
# that may involve native modules/libraries which can make it hard to uniformly
# use the latest revision.  I'm somewhat hopeful that
#
# We are currently installing an older version of mise because as of 2014-12-15
# rust nightly has problems with usage-lib related to miette.  The version
# 2024.10.1 was specifically chosen because it was the version we were using on
# the last successful provision.  The include graph for this seems potentially
# way more than we need to be able to run node, so I think we will want to drop
# this dep in the future absent some very good reason.
cargo install mise@2024.10.1

# Install node.js for scip-typescript; github lists v18 and v20 as supported;
# we are sticking with v18 for now because currently all the invocations
# hardcode v18 as well; that will need to be addressed.
SCIP_NODEJS_VERSION=nodejs@18
mise install ${SCIP_NODEJS_VERSION}

# Install scip-typescript under node.js v18
mise exec ${SCIP_NODEJS_VERSION} -- npm install -g @sourcegraph/scip-typescript

# Install scip-python under node.js v18 as well
#mise exec ${SCIP_NODEJS_VERSION} -- npm install -g @sourcegraph/scip-python
# To get my fix https://github.com/sourcegraph/scip-python/pull/150
mise exec ${SCIP_NODEJS_VERSION} -- npm install -g @asutherland/scip-python

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

# Create update script.
cat > update.sh <<"THEEND"
#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

exec > >(tee -a update-log) 2>&1

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

# Install mozsearch.
rm -rf mozsearch
git clone -b $BRANCH $MOZSEARCH_REPO mozsearch --depth=1
pushd mozsearch
git submodule init
git submodule update
popd

# Install files from the config repo.
rm -rf config
git clone -b $BRANCH $CONFIG_REPO config --depth=1

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
./update.sh master https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla
mv update-log provision-update-log-1

# Run this a second time to make sure the script is actually idempotent, so we
# don't have any surprises when the update script gets run when the VM spins up.
./update.sh master https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla
mv update-log provision-update-log-2
