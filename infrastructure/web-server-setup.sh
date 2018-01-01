#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 3 && $# != 4 ]
then
    echo "usage: $0 <config-repo-path> <index-path> <server-root> [<use_hsts>]"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)
CONFIG_FILE=$WORKING/config.json
SERVER_ROOT=$(readlink -f $3)
USE_HSTS=${4:-}

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $WORKING

sudo mkdir -p /etc/nginx/sites-enabled
sudo rm -f /etc/nginx/sites-enabled/default

rm -rf $SERVER_ROOT/docroot
mkdir -p $SERVER_ROOT/docroot
DOCROOT=$(realpath $SERVER_ROOT/docroot)

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    mkdir -p $DOCROOT/file/$TREE_NAME
    mkdir -p $DOCROOT/dir/$TREE_NAME
    ln -s $WORKING/$TREE_NAME/file $DOCROOT/file/$TREE_NAME/source
    ln -s $WORKING/$TREE_NAME/dir $DOCROOT/dir/$TREE_NAME/source

    rm -f $DOCROOT/help.html
    ln -s $WORKING/$TREE_NAME/help.html $DOCROOT
done

python $MOZSEARCH_PATH/scripts/nginx-setup.py $CONFIG_FILE $DOCROOT "$USE_HSTS" > /tmp/nginx
sudo mv /tmp/nginx /etc/nginx/sites-enabled/mozsearch.conf
sudo chmod 0644 /etc/nginx/sites-enabled/mozsearch.conf

sudo /etc/init.d/nginx reload
