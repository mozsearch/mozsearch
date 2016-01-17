#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

cat >/tmp/mozconfig <<EOF
. \$topsrcdir/browser/config/mozconfig
mk_add_options MOZ_OBJDIR=$OBJDIR
ac_add_options --enable-debug
ac_add_options --enable-optimize
ac_add_options --enable-gczeal
ac_add_options --without-ccache
EOF

# Add the special clang flags.
$MOZSEARCH_ROOT/scripts/indexer-setup.py >> /tmp/mozconfig

cd $TREE_ROOT
autoconf2.13
cd $TREE_ROOT/js/src
autoconf2.13

mkdir -p $OBJDIR
cd $OBJDIR
MOZCONFIG=/tmp/mozconfig $TREE_ROOT/configure

