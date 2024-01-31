#!/usr/bin/env bash

exec &> /home/ubuntu/index-log

set -x # Show commands
set -u # Undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

# Don't set -e here, because that will just exit the script on error. As
# this is the "root" script of the indexer process, exiting this script
# will effectively result in a silent failure. Instead, we set a trap to
# catch errors emanating from commands in this script and trigger a failure
# email.

trap 'handle_error' ERR

handle_error() {
    # In the event of failure, we will have byproducts leftover on the local
    # drive that will be lost if we don't first move them to the persistent EBS
    # store.  We create an "interrupted" parent directory for these contents in
    # order to avoid any ambiguities about what the state of the scratch drive
    # was. We only do this if we got far enough to actually start indexing.
    if [ -d "/index" ]; then
        mkdir -p /index/interrupted
        if [ -d "/mnt/index-scratch" ]; then
            mv -f /mnt/index-scratch/* /index/interrupted
        fi
        # try and get a list of open files that might have caused problems
        # unmounting /index or moving things from /mnt/index-scratch to /index.
        lsof | grep /index
    fi

    # Send failure email and shut down. Release channel failures get sent to the
    # default email address, other channel failures get sent to the author of
    # the head commit.
    $AWS_ROOT/send-failure-email.py "${EMAIL_PREFIX}" "${DEST_EMAIL}"

    # Need to terminate the script on error explicitly, otherwise bash
    # will continue the script after running the trap handler.
    exit 1
}

# Pull out the first two arguments, which are for consumption by this
# main.sh script. The rest of the arguments get passed as arguments to
# TARGETSCRIPT, so we leave them in $*
TARGETSCRIPT=$1; shift
MAXHOURS=$1; shift

# See index.sh and rebuild-blame.sh for the arguments to this script.
# This code assumes that the first two arguments for both of those scripts
# are the branch and channel values.

# Note that we set up the EMAIL_PREFIX and DEST_EMAIL variables as early
# as possible, so that they can be used by the handle_error function in
# case anything goes wrong.

SELF=$(readlink -f "$0")
BRANCH=$1
CHANNEL=$2
export AWS_ROOT=$(dirname "$SELF")

EMAIL_PREFIX="${CHANNEL}/${BRANCH}"

case "${CHANNEL}" in
    release* )
        DEST_EMAIL="searchfox-aws@mozilla.com"
        ;;
    * )
        DEST_EMAIL=$(git --git-dir="${AWS_ROOT}/../../.git" show --format="%aE" --no-patch HEAD)
        ;;
esac

mkdir -p ~/.aws
cat > ~/.aws/config <<"STOP"
[default]
region = us-west-2
STOP

# Create a crontab entry to send failure email if TARGETSCRIPT takes too long. This
# is basically a failsafe for if this instance doesn't shut down within
# 10 hours.
${AWS_ROOT}/make-crontab.py "${EMAIL_PREFIX}/timeout" "${DEST_EMAIL}" ${MAXHOURS}

# Daily cron jobs can include things like the `locate` `updatedb` script which
# can end up tying up the indexer's mount point.  These are run via `run-parts`
# which only runs executable files, so we remove that bit from all of the daily
# jobs.  We do this here as part of running the indexer rather than as part of
# provisioning because we don't want to disable the cron jobs in our local
# testing VMs, etc.
#
# We also disable weekly cron jobs because we don't need them either.  We don't
# bother with any of the longer time intervals because the directories are
# currently empty and so the globbing gets more complicated for no point.
echo "Disabling daily and weekly cron jobs for this indexing run"
sudo chmod -x /etc/cron.daily/* /etc/cron.weekly/*

echo "Creating index-scratch on local instance SSD"
${AWS_ROOT}/mkscratch.sh

# Put our tmp directory on index scratch instead of /tmp which is on our EBS
# root image and which would be both slower and has had problems with filling
# up (bug 1712578).
mkdir -p /mnt/index-scratch/tmp
export TMPDIR=/mnt/index-scratch/tmp

# Run target script with arguments supplied to this script.
${AWS_ROOT}/${TARGETSCRIPT} $*
