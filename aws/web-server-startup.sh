#!/bin/bash

exec &> ~ubuntu/startup-log

set -e
set -x

apt-get update
apt-get install -y git

# Livegrep
apt-get install -y libgflags-dev libgit2-dev libjson0-dev libboost-system-dev libboost-filesystem-dev libsparsehash-dev cmake golang g++ mercurial

# pygit2
apt-get install -y python-virtualenv python-dev libffi-dev cmake

# Other
apt-get install -y parallel realpath unzip

# Nginx
apt-get install -y nginx
mkdir -p /etc/nginx/sites-enabled
rm /etc/nginx/sites-enabled/default
cat >/etc/nginx/sites-enabled/mozsearch.conf <<"THEEND"
server {
  listen 80 default_server;
  sendfile off;

  location /static {
    root /home/ubuntu/mozsearch;
  }

  location /mozilla-central/source {
    root /home/ubuntu/docroot;
    try_files /file/$uri /dir/$uri/index.html =404;
    types { }
    default_type text/html;
    expires 1d;
    add_header Cache-Control "public";
  }

  location /mozilla-central/search {
    proxy_pass http://localhost:8000;
  }

  location /mozilla-central/define {
    proxy_pass http://localhost:8000;
  }

  location /mozilla-central/diff {
    proxy_pass http://localhost:8001;
  }

  location /mozilla-central/commit {
    proxy_pass http://localhost:8001;
  }

  location /mozilla-central/rev {
    proxy_pass http://localhost:8001;
  }

  location /mozilla-central/commit-info {
    proxy_pass http://localhost:8001;
  }

  location = / {
    root /home/ubuntu/docroot;
    try_files $uri/help.html =404;
    expires 1d;
    add_header Cache-Control "public";
  }
}
THEEND
chmod 0644 /etc/nginx/sites-enabled/mozsearch.conf

/etc/init.d/nginx reload

while true
do
    COUNT=$(lsblk | grep xvdf | wc -l)
    if [ $COUNT -eq 1 ]
    then break
    fi
    sleep 1
done

echo "Index volume detected"

mkdir ~ubuntu/index
mount /dev/xvdf ~ubuntu/index

echo "Finished installation"

cat > ~ubuntu/web-server <<"THEEND"
#!/bin/bash

set -e
set -x

cd $HOME

exec &> $HOME/web-server-log

#SETCHANNEL
if [ "x$CHANNEL" = "x" ]
then
    CHANNEL=release
fi

echo "Channel is $CHANNEL"

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

wget -q https://index.taskcluster.net/v1/task/gecko.v2.mozilla-central.nightly.latest.firefox.linux64-opt/artifacts/public/build/jsshell-linux-x86_64.zip
mkdir js
pushd js
unzip ../jsshell-linux-x86_64.zip
popd

export LD_LIBRARY_PATH=\$HOME/js
export JS=\$HOME/js/js

virtualenv env
VENV=$(realpath env)

# Install pygit2
wget -q https://github.com/libgit2/libgit2/archive/v0.24.0.tar.gz
tar xf v0.24.0.tar.gz
pushd libgit2-0.24.0
cmake . -DCMAKE_INSTALL_PREFIX=$VENV
make
make install
popd
LIBGIT2=$VENV LDFLAGS="-Wl,-rpath='$VENV/lib',--enable-new-dtags $LDFLAGS" $VENV/bin/pip install pygit2

git clone https://github.com/livegrep/livegrep
pushd livegrep
make bin/codesearch
popd
export CODESEARCH=$HOME/livegrep/bin/codesearch

BRANCH=master
if [ $CHANNEL != release ]
then
    BRANCH=$CHANNEL
fi

git clone -b $BRANCH https://github.com/bill-mccloskey/mozsearch
pushd mozsearch
git submodule init
git submodule update
popd

pushd mozsearch/tools
cargo build --release
popd

mkdir -p docroot/file/mozilla-central
mkdir -p docroot/dir/mozilla-central
ln -s $HOME/index/file docroot/file/mozilla-central/source
ln -s $HOME/index/dir docroot/dir/mozilla-central/source
ln -s $HOME/index/help.html docroot

cat >$HOME/config.json <<OTHEREND
{
  "mozsearch_path": "$HOME/mozsearch",

  "mozilla-central": {
    "index_path": "$HOME/index",
    "repo_path": "$HOME/index/gecko-dev",
    "blame_repo_path": "$HOME/index/gecko-blame",
    "objdir_path": ""
  }
}
OTHEREND

cd mozsearch

nohup $VENV/bin/python router/router.py $HOME/config.json > $HOME/router.log 2> $HOME/router.err < /dev/null &

nohup tools/target/release/web-server $HOME/config.json > $HOME/rust-server.log 2> $HOME/rust-server.err < /dev/null &

THEEND

chmod +x ~ubuntu/web-server
su - -c ~ubuntu/web-server ubuntu

echo "Finished"
