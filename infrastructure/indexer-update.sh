#!/usr/bin/env bash
#
# This script is run on the indexer by the `update.sh` script created by the
# provisioning process.  Its purpose is to:
# 1. Download/update dependencies that change frequently and need to be
#    up-to-date for indexing/analysis reasons (ex: spidermonkey for JS, rust).
# 2. Perform the build steps for mozsearch.
#
# When developing, this is also a good place to:
# - Install any additional dependencies you might need.
# - Perform any new build steps your changes need.
#
# However, when it comes time to land, it's preferable to make sure that
# dependencies that don't change should just be installed once at provisioning
# time.
#

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Install Nix-provided packages
sudo -i nix profile add "$(pwd)/mozsearch#indexerPackages" --accept-flake-config --print-build-logs --priority 3

# Install SpiderMonkey.
rm -rf target.jsshell.zip js
wget -nv https://firefox-ci-tc.services.mozilla.com/api/index/v1/task/gecko.v2.mozilla-central.latest.firefox.linux64-opt/artifacts/public/build/target.jsshell.zip
mkdir js
pushd js
unzip ../target.jsshell.zip
sudo install js /usr/local/bin
sudo install *.so /usr/local/lib
sudo ldconfig
popd

PYMODULES=$HOME/pymodules

# Delete the temp dir if IDL parsers are older than a day (in minutes to avoid
# quantization weirdness).  We'll also try and delete the dir if the file just
# doesn't exist, which also means if the directory doesn't exist.  (We could
# have instead done `-mmin +1440` for affirmative confirmation it's old, but
# since our next check is just for the existence of the directory, this is least
# likely to result in weirdness.)
if [ ! "$(find $PYMODULES/xpidl.py -mmin -1440)" ]; then
    rm -rf $PYMODULES
fi

# download/copy as needed
if [ ! -d "${PYMODULES}" ]; then
    mkdir "${PYMODULES}"
    pushd "${PYMODULES}"
    wget "https://github.com/mozilla-firefox/firefox/raw/refs/heads/main/xpcom/idl-parser/xpidl/xpidl.py"
    wget "https://github.com/mozilla-firefox/firefox/raw/refs/heads/main/dom/bindings/parser/WebIDL.py"
    mkdir ply
    pushd ply
    for PLYFILE in __init__.py lex.py yacc.py; do
        wget "https://github.com/mozilla-firefox/firefox/raw/refs/heads/main/third_party/python/ply/ply/${PLYFILE}"
    done
    popd
    popd
fi
