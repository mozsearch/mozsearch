#!/usr/bin/env python3

# The size of a channel's index volume, in GiB (see attach-index-volume.py),
# which the indexer's /index partition has too, so that the index can be built
# in place and copied to the volume at the block level (see mkscratch.sh and
# index.sh).
# Usage: index_volume.py <channel>

import sys


def volume_size(channel):
    # We've been using 300G for quite some time but with the addition of
    # esr140 to config2, we hit the 300G limit; the math indicates we probably
    # need at least 305G before the nginx cache which we cap at 20G.  To
    # provide headroom, I'm going to use 400 for release2 but we can revisit as
    # we rebalance things.
    #
    # release3 also takes ~300GB, where each mozilla-esr* take 25GB to 40GB,
    # and there are 8 repositories. comm-* don't take much space.
    if channel == 'release2' or channel == 'release3':
        return 400
    return 300


if __name__ == '__main__':
    print(volume_size(sys.argv[1]))
