#!/usr/bin/env bash
# Usage: gzip-with-markers.sh <file>...: gzip each file, leaving in its place
# a zero-length marker with its gzipped file's times (for nginx's try_files),
# as `gzip -f F && touch -r F.gz F` would, with one gzip and one python
# process for all of them, rather than three processes for each.
set -eu
gzip -f -- "$@"
python3 -c '
import os, sys
for f in sys.argv[1:]:
    s = os.stat(f + ".gz")
    open(f, "w").close()
    os.utime(f, ns=(s.st_atime_ns, s.st_mtime_ns))
' "$@"
