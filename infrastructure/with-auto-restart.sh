#!/usr/bin/env bash

MAX_RETRY=10
ITER=1

MOZSEARCH_PATH=$1
shift
CHANNEL=$1
shift
SERVER_NAME=$1
shift
DEST_EMAIL=$1
shift
ERROR_LOG_FILE=$1
shift

while true; do
    "$@"

    EXIT_CODE=$?

    echo "!!!! PROCESS EXIT WITH $EXIT_CODE !!!!" 1>&2

    if [[ $EXIT_CODE -eq 0 ]]; then
        exit $EXIT_CODE
    fi

    if [[ $EXIT_CODE -eq 143 ]]; then
        # SIGTERM, used by web-server-run.sh before starting a new one,
        # and also used on the instance shutdown.
        # We don't auto-restart.
        exit $EXIT_CODE
    fi

    # NOTE: We restart on SIGKILL (137), given OOM-killer uses it.
    # NOTE: We also restart on 101, which is used by Rust panic!().

    if [[ $ITER -le $MAX_RETRY ]]; then
        echo "!!!! RETRYING ($ITER/$MAX_RETRY) !!!!" 1>&2
        ITER=$(($ITER + 1))
    else
        echo "!!!! GIVING UP !!!!" 1>&2

        $MOZSEARCH_PATH/infrastructure/aws/send-server-failure-email.py $CHANNEL $SERVER_NAME $DEST_EMAIL $ERROR_LOG_FILE

        exit $EXIT_CODE
    fi
done
