#!/bin/bash

exec &> /root/startup-log

set -e
set -x

apt-get update
apt-get autoremove -y

apt-get install -y git

# Firefox: https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
apt-get install -y zip unzip mercurial g++ make autoconf2.13 yasm libgtk2.0-dev libglib2.0-dev libdbus-1-dev libdbus-glib-1-dev libasound2-dev libcurl4-openssl-dev libiw-dev libxt-dev mesa-common-dev libgstreamer0.10-dev libgstreamer-plugins-base0.10-dev libpulse-dev m4 flex ccache libgconf2-dev

# Livegrep
apt-get install -y libgflags-dev libgit2-dev libjson0-dev libboost-system-dev libboost-filesystem-dev libsparsehash-dev cmake golang

# Other
apt-get install -y parallel realpath

echo "Finished installation"

mkdir /mnt/index-tmp
chown ubuntu.ubuntu /mnt/index-tmp

mkfs -t ext4 /dev/xvdc
mkdir /index
mount /dev/xvdc /index
chown ubuntu.ubuntu /index

cat > ~ubuntu/indexer <<THEEND
#!/bin/bash

set -e
set -x

export INDEX_TMP=/mnt/index-tmp

cd \$INDEX_TMP

exec &> ~ubuntu/index-log

wget http://ftp.mozilla.org/pub/mozilla.org/firefox/tinderbox-builds/mozilla-central-linux64-pgo/latest/jsshell-linux-x86_64.zip
mkdir js
pushd js
unzip ../jsshell-linux-x86_64.zip
popd

export LD_LIBRARY_PATH=\$INDEX_TMP/js
export JS=\$INDEX_TMP/js/js

git clone https://github.com/mozilla/gecko-dev
mv gecko-dev mozilla-central

git clone https://github.com/livegrep/livegrep
pushd livegrep
make
popd
export CODESEARCH=\$INDEX_TMP/livegrep/bin/codesearch

git clone https://github.com/bill-mccloskey/mozsearch
\$INDEX_TMP/mozsearch/mkindex \$INDEX_TMP/mozsearch /index \$INDEX_TMP
THEEND

chmod +x ~ubuntu/indexer
su - -c ~ubuntu/indexer ubuntu

echo "Finished"
