#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Fail on reprovisioning attempts, as we don't support them
if [ -f $HOME/.provisioned ]; then
    echo "Sorry! Re-provisioning is not supported. Please destroy your vagrant box and re-create/provision it from scratch, or manually apply changes."
    echo "If you want to manually apply changes, here is the commit at which this box was last provisioned:"
    cat $HOME/.provisioned
    exit 1
fi
git -C /vagrant log -1 > $HOME/.provisioned

# Bug 1766697:
# Compensate for UID/GID mis-matches that freak out git after
# https://github.blog/2022-04-12-git-security-vulnerability-announced/.
# We could alternately fix the problem by involving vagrant-bindfs.
git config --global --add safe.directory /vagrant

# Install SpiderMonkey.
rm -rf jsshell-linux-x86_64.zip js
wget -nv https://firefox-ci-tc.services.mozilla.com/api/index/v1/task/gecko.v2.mozilla-central.latest.firefox.linux64-opt/artifacts/public/build/target.jsshell.zip
mkdir js
pushd js
unzip ../target.jsshell.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd
