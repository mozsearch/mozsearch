#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

MOZSEARCH_REPO="${MOZSEARCH_REPO:-https://github.com/mozsearch/mozsearch}"
MOZSEARCH_BRANCH="${MOZSEARCH_BRANCH:-master}"
MOZSEARCH_CONFIG_REPO="${MOZSEARCH_CONFIG_REPO:-https://github.com/mozsearch/mozsearch-mozilla}"
MOZSEARCH_CONFIG_BRANCH="${MOZSEARCH_CONFIG_BRANCH:-master}"

# Nginx
sudo apt-get install -y nginx

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
# Note: This seems needlessly wasteful but I'm not going to change this while
# changing other things.
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
mozsearch/infrastructure/web-server-update.sh

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
