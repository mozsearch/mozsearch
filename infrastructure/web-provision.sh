#!/bin/bash

set -e
set -x

sudo apt-get update
sudo apt-get install -y git

# Livegrep
sudo apt-get install -y libgflags-dev libgit2-dev libjson0-dev libboost-system-dev libboost-filesystem-dev libsparsehash-dev cmake golang g++ mercurial

# pygit2
sudo apt-get install -y python-virtualenv python-dev libffi-dev cmake

# Other
sudo apt-get install -y parallel realpath unzip

# Nginx
sudo apt-get install -y nginx

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install codesearch.
git clone https://github.com/livegrep/livegrep
pushd livegrep
make bin/codesearch
install bin/codesearch /usr/local/bin
popd

# Install pygit2.
wget -q https://github.com/libgit2/libgit2/archive/v0.24.0.tar.gz
tar xf v0.24.0.tar.gz
pushd libgit2-0.24.0
cmake .
make
make install
popd
sudo ldconfig
sudo pip install pygit2
