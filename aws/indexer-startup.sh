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

wget https://s3.amazonaws.com/aws-cloudwatch/downloads/latest/awslogs-agent-setup.py
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
apt-get install -y python-dev libffi-dev

# Setup direct links to clang
update-alternatives --install /usr/bin/llvm-config llvm-config /usr/bin/llvm-config-3.6 360
update-alternatives --install /usr/bin/clang clang /usr/bin/clang-3.6 360
update-alternatives --install /usr/bin/clang++ clang++ /usr/bin/clang++-3.6 360

echo "Finished installation"

mkdir /mnt/index-tmp
chown ubuntu.ubuntu /mnt/index-tmp

cat > ~ubuntu/indexer <<"THEEND"
#!/bin/bash

set -e
set -x

exec &> ~ubuntu/index-log

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

wget https://index.taskcluster.net/v1/task/gecko.v2.mozilla-central.nightly.latest.firefox.linux64-opt/artifacts/public/build/jsshell-linux-x86_64.zip
mkdir js
pushd js
unzip ../jsshell-linux-x86_64.zip
popd

export LD_LIBRARY_PATH=$INDEX_TMP/js
export JS=$INDEX_TMP/js/js

hg clone https://hg.mozilla.org/mozilla-central

git clone https://github.com/livegrep/livegrep
pushd livegrep
make
popd
export CODESEARCH=$INDEX_TMP/livegrep/bin/codesearch

virtualenv env
VENV=$(realpath env)

# Install AWS scripts
$VENV/bin/pip install boto3

# Install pygit2
wget https://github.com/libgit2/libgit2/archive/v0.24.0.tar.gz
tar xf v0.24.0.tar.gz
pushd libgit2-0.24.0
cmake . -DCMAKE_INSTALL_PREFIX=$VENV
make
make install
popd
LIBGIT2=$VENV LDFLAGS="-Wl,-rpath='$VENV/lib',--enable-new-dtags $LDFLAGS" $VENV/bin/pip install pygit2

git clone https://github.com/bill-mccloskey/mozsearch

pushd mozsearch/clang-plugin
make
popd

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

pushd /index
git clone https://github.com/mozilla/gecko-dev

wget https://s3-us-west-2.amazonaws.com/blame-repo/gecko-blame.tar
tar xf gecko-blame.tar
popd

export VENV
export HG_ROOT=$INDEX_TMP/mozilla-central
export TREE_ROOT=/index/gecko-dev
export TREE_REV=$(cd $TREE_ROOT; git show-ref -s --head HEAD)
export BLAME_ROOT=/index/gecko-blame
export OBJDIR=$INDEX_TMP/objdir
export INDEX_ROOT=/index
export MOZSEARCH_ROOT=$INDEX_TMP/mozsearch

$INDEX_TMP/mozsearch/update-repos

$INDEX_TMP/mozsearch/mkindex

date
echo "Indexing complete"

sudo umount /index

$VENV/bin/python $AWS_ROOT/detach-volume.py $EC2_INSTANCE_ID $VOLUME_ID
$VENV/bin/python $AWS_ROOT/swap-web-server.py $CHANNEL $EC2_INSTANCE_ID $VOLUME_ID

# Give logger time to catch up
sleep 30
$VENV/bin/python $AWS_ROOT/terminate-indexer.py $EC2_INSTANCE_ID
popd

THEEND

chmod +x ~ubuntu/indexer
su - -c ~ubuntu/indexer ubuntu

echo "Finished"
