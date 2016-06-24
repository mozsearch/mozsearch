#!/bin/bash

# Not sure if this does anything, but I want to make sure the instance
# is in a good state.
sleep 30

exec &> /root/startup-log

set -e
set -x

cat > ./cloudwatch.cfg <<"THEEND"
[general]
state_file = /var/awslogs/state/agent-state

[/root/startup-log]
file = /root/startup-log
log_group_name = /root/startup-log
log_stream_name = {instance_id}

[/home/ubuntu/index-log]
file = /home/ubuntu/index-log
log_group_name = /home/ubuntu/index-log
log_stream_name = {instance_id}
THEEND

date

wget -q https://s3.amazonaws.com/aws-cloudwatch/downloads/latest/awslogs-agent-setup.py
chmod +x ./awslogs-agent-setup.py
./awslogs-agent-setup.py -n -r us-west-2 -c ./cloudwatch.cfg

apt-get update
apt-get autoremove -y

apt-get install -y git

# Firefox: https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
apt-get install -y zip unzip mercurial g++ make autoconf2.13 yasm libgtk2.0-dev libgtk-3-dev libglib2.0-dev libdbus-1-dev libdbus-glib-1-dev libasound2-dev libcurl4-openssl-dev libiw-dev libxt-dev mesa-common-dev libgstreamer0.10-dev libgstreamer-plugins-base0.10-dev libpulse-dev m4 flex ccache libgconf2-dev clang-3.6 libclang-3.6-dev

# Livegrep
apt-get install -y libgflags-dev libgit2-dev libjson0-dev libboost-system-dev libboost-filesystem-dev libsparsehash-dev cmake golang

# Other
apt-get install -y parallel realpath source-highlight python-virtualenv python-dev

# pygit2
apt-get install -y python-dev libffi-dev cmake

# Setup direct links to clang
update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config-3.6 360
update-alternatives --install /usr/bin/clang clang /usr/bin/clang-3.6 360
update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-3.6 360

echo "Finished installation"
date

mkdir /mnt/index-tmp
chown ubuntu.ubuntu /mnt/index-tmp

cat > ~ubuntu/indexer <<"THEEND"
#!/bin/bash

set -e
set -x

exec &> ~ubuntu/index-log

date

EC2_INSTANCE_ID=$(wget -q -O - http://instance-data/latest/meta-data/instance-id)

mkdir ~/.aws
cat > ~/.aws/config <<"STOP"
[default]
region = us-west-2
STOP

export INDEX_TMP=/mnt/index-tmp

cd $INDEX_TMP

#SETCHANNEL
if [ "x$CHANNEL" = "x" ]
then
    CHANNEL=release
fi

echo "Channel is $CHANNEL"

# Install Rust.
curl -sSf https://static.rust-lang.org/rustup.sh | sh

# Install SpiderMonkey.
wget -q https://index.taskcluster.net/v1/task/gecko.v2.mozilla-central.nightly.latest.firefox.linux64-opt/artifacts/public/build/jsshell-linux-x86_64.zip
mkdir js
pushd js
unzip ../jsshell-linux-x86_64.zip
popd

export LD_LIBRARY_PATH=$INDEX_TMP/js
export JS=$INDEX_TMP/js/js

date

git clone https://github.com/livegrep/livegrep
pushd livegrep
make
popd
export PATH=$PATH:$INDEX_TMP/livegrep/bin

date

virtualenv env
VENV=$(realpath env)

# Install AWS scripts
$VENV/bin/pip install boto3

# Install pygit2
wget -q https://github.com/libgit2/libgit2/archive/v0.24.0.tar.gz
tar xf v0.24.0.tar.gz
pushd libgit2-0.24.0
cmake . -DCMAKE_INSTALL_PREFIX=$VENV
make
make install
popd
LIBGIT2=$VENV LDFLAGS="-Wl,-rpath='$VENV/lib',--enable-new-dtags $LDFLAGS" $VENV/bin/pip install pygit2

date

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
export MOZSEARCH_ROOT=$INDEX_TMP/mozsearch

pushd mozsearch/clang-plugin
make
popd

date

pushd mozsearch/tools
cargo build --release --verbose
popd

date

export AWS_ROOT=$(realpath mozsearch/aws)
VOLUME_ID=$($VENV/bin/python $AWS_ROOT/attach-index-volume.py $CHANNEL $EC2_INSTANCE_ID)

while true
do
    COUNT=$(lsblk | grep xvdf | wc -l)
    if [ $COUNT -eq 1 ]
    then break
    fi
    sleep 1
done

echo "Index volume detected"

sudo mkfs -t ext4 /dev/xvdf
sudo mkdir /index
sudo mount /dev/xvdf /index
sudo chown ubuntu.ubuntu /index

export VENV
export CONFIG_FILE=$INDEX_TMP/config.json

cat >$CONFIG_FILE <<OTHEREND
{
  "mozsearch_path": "$MOZSEARCH_ROOT",
  "livegrep_path": "/index",

  "repos": {
    "mozilla-central": {
      "index_path": "/index/mozilla-central",
      "repo_path": "/index/mozilla-central/gecko-dev",
      "hg_path": "$INDEX_TMP/mozilla-central",
      "blame_repo_path": "/index/mozilla-central/gecko-blame",
      "objdir_path": "$INDEX_TMP/mozilla-central/objdir"
    }
  }
}
OTHEREND

for TREE_NAME in $($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos)
do
   .  $MOZSEARCH_ROOT/scripts/load-vars.sh $CONFIG_FILE $TREE_NAME
    mkdir -p $INDEX_ROOT

    date
    $MOZSEARCH_ROOT/repos-setup/$TREE_NAME/setup
done

date

for TREE_NAME in $($MOZSEARCH_ROOT/scripts/read-json.py $CONFIG_FILE repos)
do
    date
    $MOZSEARCH_ROOT/update-repos $CONFIG_FILE $TREE_NAME

    date
    $MOZSEARCH_ROOT/mkindex $CONFIG_FILE $TREE_NAME
done

date
$MOZSEARCH_ROOT/scripts/build-codesearch.py $CONFIG_FILE

date
echo "Indexing complete"

sudo umount /index

$VENV/bin/python $AWS_ROOT/detach-volume.py $EC2_INSTANCE_ID $VOLUME_ID
$VENV/bin/python $AWS_ROOT/swap-web-server.py $CHANNEL $EC2_INSTANCE_ID $VOLUME_ID

gzip -k ~ubuntu/index-log
$VENV/bin/python $AWS_ROOT/upload.py ~ubuntu/index-log.gz indexer-logs `date -Iminutes`

# Give logger time to catch up
sleep 30
$VENV/bin/python $AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
popd

THEEND

chmod +x ~ubuntu/indexer
su - -c ~ubuntu/indexer ubuntu

echo "Finished"
