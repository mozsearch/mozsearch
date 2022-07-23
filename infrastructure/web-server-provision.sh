#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Nginx
sudo apt-get install -y nginx

# Create update script.
cat > update.sh <<"THEEND"
#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

exec &> update-log

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
git clone $MOZSEARCH_REPO mozsearch
pushd mozsearch
git checkout $MOZSEARCH_REV
git submodule init
git submodule update
popd

# Install files from the config repo.
rm -rf config
git clone $CONFIG_REPO config
pushd config
git checkout $CONFIG_REV
popd

date

# Let mozsearch tell us what commonly changing dependencies to install plus
# perform any build steps.
mozsearch/infrastructure/web-server-update.sh

date
THEEND

chmod +x update.sh
