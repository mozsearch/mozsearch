#!/bin/bash

set -e
set -x

if [ $# != 2 ]
then
    echo "usage: $0 <config-repo-path> <working-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

CONFIG_REPO=$(readlink -f $1)
WORKING=$(readlink -f $2)
CONFIG_FILE=$WORKING/config.json

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $WORKING

sudo mkdir -p /etc/nginx/sites-enabled
sudo rm -f /etc/nginx/sites-enabled/default

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    mkdir -p docroot/file/$TREE_NAME
    mkdir -p docroot/dir/$TREE_NAME
    ln -s $WORKING/$TREE_NAME/file docroot/file/$TREE_NAME/source
    ln -s $WORKING/$TREE_NAME/dir docroot/dir/$TREE_NAME/source

    rm -f docroot/help.html
    ln -s $WORKING/$TREE_NAME/help.html docroot
done

DOCROOT=$(realpath docroot)
python $MOZSEARCH_PATH/scripts/nginx-setup.py $CONFIG_FILE $DOCROOT > /tmp/nginx
sudo mv /tmp/nginx /etc/nginx/sites-enabled/mozsearch.conf
sudo chmod 0644 /etc/nginx/sites-enabled/mozsearch.conf

sudo /etc/init.d/nginx reload
