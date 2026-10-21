#!/usr/bin/env python3

# The size of a channel's index volume, in GiB (see attach-index-volume.py),
# which the indexer's /index partition has too, so that the index can be built
# in place and copied to the volume at the block level (see mkscratch.sh and
# index.sh).
# Usage: index_volume.py <channel> <config-file>

import json
import sys

# A config file can ask for a different size with "index_volume_gb" (in GiB, as
# EBS sizes are), ex: config8.json, whose firefox-disco tree's full history
# (see its update-history.sh) is ~120 GiB in firefox-shared.  (Like the web
# server's instance type, see trigger-web-server.py, within limits, since the
# volume lives as long as the web server serving it.)
MAX_VOLUME_SIZE = 1000


def volume_size(channel, config_file):
    config = json.load(open(config_file))
    if 'index_volume_gb' in config:
        requested = config['index_volume_gb']
        if not isinstance(requested, int) or not 0 < requested <= MAX_VOLUME_SIZE:
            raise Exception(f'index_volume_gb must be a number of GiB up to {MAX_VOLUME_SIZE}, not {requested!r}')
        return requested

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
    print(volume_size(sys.argv[1], sys.argv[2]))
