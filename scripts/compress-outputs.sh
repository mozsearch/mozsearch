#!/usr/bin/env bash

# This will be chatty, so we don't use `set -x`
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Compress all of our HTML and raw-analysis files which end up being very large
# and only exist to be served via nginx and therefore can be pre-compressed,
# resulting in a net win for everyone.
#
# Limitations in our use of try_files by nginx and the `gzip_static` logic mean
# that:
# - In cases where we're using try_files we need to have the normal
#   non-gz-suffixed file on disk for try_files to be able to identify what
#   directory the file is in from its search list.  This need not be the
#   original file, so we can be tricky and just create a zero-length version of
#   the file because no one will read it.
# - The gzipped versions of the files need to just have the straightforward .gz
#   suffix.
#
# If you were thinking "What if the tree actually contains both FOO and FOO.gz?"
# then you win a prize.  And that prize is either coming up with an elegant fix
# to the problem or being party to the inelegant fix we depend on.
#
# It turns out mozilla-central has exactly this case in the guise of
# `devtools/client/styleeditor/test/simple.css` and its gzipped twin
# `devtools/client/styleeditor/test/simple.css.gz`.  Right now (pre this patch)
# if you view the ".gz" file you'll see a useless single gibberish line of text.
#
# Our fix:
# - We observe that if we process the existing "FOO.gz" file before "FOO", then
#   when FOO's gzipping overwrites "FOO.gz", there's no harm because it's
#   overwriting a zero-length file that only needed to exist so its filename
#   existed.
# - We observe that `sort -r` provides this ordering.
# - We use `gzip -f` so that gzip doesn't care when it overwrites the zero
#   length file.
# - We note that although we try and be responsible with the timestamps in this
#   script, they don't actually matter at all because they're the timestamps for
#   generated HTML files and don't have timestamps corresponding to the
#   underlying revision controlled files.  (Also, the timestamp that gets served
#   to users is that of the gzip file anyways and so the overwrite case doesn't
#   do any harm there either.)

echo "Compressing files in ${INDEX_ROOT}/file/ with zero-length marker for try_files"
pushd ${INDEX_ROOT}/file/

find . -type f | sort -r | while read -r file; do
  gzip -f "$file"
  touch -r "$file".gz "$file"
done
popd

echo "Compressing files in ${INDEX_ROOT}/dir/ with zero-length marker for try_files"
pushd ${INDEX_ROOT}/dir/
find . -type f | sort -r | while read -r file; do
  gzip -f "$file"
  touch -r "$file".gz "$file"
done
popd

echo "Compressing files in ${INDEX_ROOT}/analysis/ without zero-length marker"
pushd ${INDEX_ROOT}/analysis/
find . -type f | sort -r | while read -r file; do
  gzip -f "$file"
done
popd