#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Run the update script for a side effect of downloading the crates.io
# dependencies ahead of time since we're seeing intermittent network problems
# downloading crates in https://bugzilla.mozilla.org/show_bug.cgi?id=1720037.
#
# Note that because the update script fully deletes the mozsearch directory,
# this really is just:
# - Validating the image can compile and use rust and clang correctly.
# - Caching some crates in `~/.cargo`.
./update.sh master https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla
mv update-log provision-update-log-1

# Run this a second time to make sure the script is actually idempotent, so we
# don't have any surprises when the update script gets run when the VM spins up.
./update.sh master https://github.com/mozsearch/mozsearch https://github.com/mozsearch/mozsearch-mozilla
mv update-log provision-update-log-2
