#!/bin/bash

exec &> ~ubuntu/startup-log

set -e
set -x

apt-get update
apt-get autoremove -y

apt-get install -y git

# Livegrep
apt-get install -y libgflags-dev libgit2-dev libjson0-dev libboost-system-dev libboost-filesystem-dev libsparsehash-dev cmake golang g++ mercurial

# Other
apt-get install -y parallel realpath unzip

echo "Finished installation"

cat > ~ubuntu/web-server <<THEEND
#!/bin/bash

set -e
set -x

cd ~ubuntu

exec &> ~ubuntu/web-server-log

wget http://ftp.mozilla.org/pub/mozilla.org/firefox/tinderbox-builds/mozilla-central-linux64-pgo/latest/jsshell-linux-x86_64.zip
mkdir js
pushd js
unzip ../jsshell-linux-x86_64.zip
popd

export LD_LIBRARY_PATH=\$HOME/js
export JS=\$HOME/js/js

git clone https://github.com/livegrep/livegrep
pushd livegrep
make
popd
export CODESEARCH=\$HOME/livegrep/bin/codesearch

git clone https://github.com/bill-mccloskey/mozsearch
THEEND

chmod +x ~ubuntu/web-server
su - -c ~ubuntu/web-server ubuntu

echo "Finished"
