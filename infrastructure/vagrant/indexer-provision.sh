#!/bin/bash

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

pushd /vagrant/clang-plugin
make
popd

pushd /vagrant/tools
cargo build --release --verbose
popd
