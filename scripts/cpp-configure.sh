#!/bin/bash

set -e # Errors are fatal
set -x # Show commands

cat > $INDEX_ROOT/mozconfig <<EOF
. \$topsrcdir/browser/config/mozconfig
mk_add_options MOZ_OBJDIR=$OBJDIR
ac_add_options --enable-debug
ac_add_options --enable-optimize
ac_add_options --enable-gczeal
ac_add_options --without-ccache
EOF

# Add the special clang flags.
$MOZSEARCH_ROOT/scripts/indexer-setup.py >> $INDEX_ROOT/mozconfig
