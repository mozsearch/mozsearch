#!/bin/bash

set -e
set -x

if [ $# != 3 ]
then
    echo "usage: $0 <config-repo-path> <base-path> <temp-path>"
    exit 1
fi

MOZSEARCH_PATH=$(cd $(dirname "$0") && git rev-parse --show-toplevel)

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $BASE $TEMP

sudo mkdir -p /etc/nginx/sites-enabled
sudo rm /etc/nginx/sites-enabled/default

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
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
