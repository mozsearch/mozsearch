#!/bin/bash

set -e
set -x

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <base-path> <temp-path>"
    exit 1
fi

SCRIPT_PATH=$(readlink -f "$0")
MOZSEARCH_PATH=$(dirname "$SCRIPT_PATH")/..
MOZSEARCH_ROOT=$MOZSEARCH_PATH

CONFIG_REPO=$(readlink -f $1)
BASE=$(readlink -f $2)
TEMP=$(readlink -f $3)

CONFIG_FILE=$TEMP/config.json

export MOZSEARCH_PATH
export BASE
export TEMP
envsubst < $CONFIG_REPO/config.json > $CONFIG_FILE

sudo mkdir -p /etc/nginx/sites-enabled
sudo rm /etc/nginx/sites-enabled/default

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE repos)
do
    mkdir -p docroot/file/$TREE_NAME
    mkdir -p docroot/dir/$TREE_NAME
    ln -s $HOME/index/$TREE_NAME/file docroot/file/$TREE_NAME/source
    ln -s $HOME/index/$TREE_NAME/dir docroot/dir/$TREE_NAME/source
done

ln -s $HOME/index/mozilla-central/help.html docroot

DOCROOT=$(realpath docroot)
python $MOZSEARCH_PATH/scripts/nginx-setup.py $CONFIG_FILE $DOCROOT > /tmp/nginx
sudo mv /tmp/nginx /etc/nginx/sites-enabled/mozsearch.conf
sudo chmod 0644 /etc/nginx/sites-enabled/mozsearch.conf

sudo /etc/init.d/nginx reload
