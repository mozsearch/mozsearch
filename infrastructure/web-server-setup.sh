#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# != 4 -a $# != 5 -a $# != 6 ]
then
    echo "usage: $0 <config-repo-path> <config-file-name> <index-path> <server-root> [<use_hsts>] [nginx-cache-dir]"
    exit 1
fi

MOZSEARCH_PATH=$(readlink -f $(dirname "$0")/..)

CONFIG_REPO=$(readlink -f $1)
CONFIG_INPUT="$2"
WORKING=$(readlink -f $3)
CONFIG_FILE=$WORKING/config.json
SERVER_ROOT=$(readlink -f $4)
USE_HSTS=${5:-}
NGINX_CACHE_DIR=${6:-}

$MOZSEARCH_PATH/scripts/generate-config.sh $CONFIG_REPO $CONFIG_INPUT $WORKING

sudo mkdir -p /etc/nginx/sites-enabled
sudo rm -f /etc/nginx/sites-enabled/default

rm -rf $SERVER_ROOT/docroot
mkdir -p $SERVER_ROOT/docroot
DOCROOT=$(realpath $SERVER_ROOT/docroot)

DEFAULT_TREE_NAME=$($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE default_tree)

for TREE_NAME in $($MOZSEARCH_PATH/scripts/read-json.py $CONFIG_FILE trees)
do
    mkdir -p $DOCROOT/file/$TREE_NAME
    mkdir -p $DOCROOT/dir/$TREE_NAME
    mkdir -p $DOCROOT/raw-analysis/$TREE_NAME
    mkdir -p $DOCROOT/file-lists/$TREE_NAME/file-lists
    ln -s $WORKING/$TREE_NAME/file $DOCROOT/file/$TREE_NAME/source
    ln -s $WORKING/$TREE_NAME/dir $DOCROOT/dir/$TREE_NAME/source
    ln -s $WORKING/$TREE_NAME/analysis $DOCROOT/raw-analysis/$TREE_NAME/raw-analysis
    for FILE_LIST in repo-files objdir-files; do
        ln -s $WORKING/$TREE_NAME/$FILE_LIST $DOCROOT/file-lists/$TREE_NAME/file-lists/$FILE_LIST
    done

    # Only update the help file if no default tree was specified OR
    # The tree was specified and this is that tree.
    if [ -z "$DEFAULT_TREE_NAME" -o "$DEFAULT_TREE_NAME" == "$TREE_NAME" ]
    then
        rm -f $DOCROOT/help.html
        ln -s $WORKING/$TREE_NAME/help.html $DOCROOT
    fi
done

python $MOZSEARCH_PATH/scripts/nginx-setup.py $CONFIG_FILE $DOCROOT "$USE_HSTS" "$NGINX_CACHE_DIR" > /tmp/nginx
sudo mv /tmp/nginx /etc/nginx/sites-enabled/mozsearch.conf
sudo chmod 0644 /etc/nginx/sites-enabled/mozsearch.conf

sudo /etc/init.d/nginx reload
