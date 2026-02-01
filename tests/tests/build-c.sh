#!/usr/bin/env bash

set -x # Show commands
set -eu # Errors/undefined vars are fatal
set -o pipefail # Check all commands in a pipeline

if [ $# -ne 3 ]
then
    echo "Usage: build-c.sh src-file objdir platforms"
    exit 1
fi

SRC_FILE=$1
OBJDIR=$2
PLATFORMS=$3

for PLATFORM in ${PLATFORMS}
do
    if echo $SRC_FILE | grep $OBJDIR > /dev/null; then
        LOCAL_FILE=$(echo $SRC_FILE | sed -e "s|$OBJDIR|__GENERATED__|")
        OBJ_FILE=$OBJDIR/__${PLATFORM}__/${LOCAL_FILE%%.cpp}.o
    else
        LOCAL_FILE=$SRC_FILE
        OBJ_FILE=$OBJDIR/__${PLATFORM}__/${LOCAL_FILE%%.cpp}.o
    fi
    mkdir -p $(dirname $OBJ_FILE)

    MACRO_PLATFORM=$(echo ${PLATFORM} | cut -d - -f 1)
    if echo ${PLATFORM} | grep '.-opt$'; then
        EXTRA_OPTS=""
    else
        EXTRA_OPTS="-DDEBUG"
    fi
    MOZSEARCH_PLATFORM=$PLATFORM $CXX -DTEST_MACRO1 -DTEST_MACRO2 -DTARGET_$MACRO_PLATFORM ${EXTRA_OPTS} -DTEST_MACRO_INCLUDE='"nsISupports.h"' $SRC_FILE -std=c++17 -I . -I $OBJDIR -c -o $OBJ_FILE -Wall
    mkdir -p  $(dirname $INDEX_ROOT/analysis-$PLATFORM/$LOCAL_FILE)
    mv $INDEX_ROOT/analysis/$LOCAL_FILE $INDEX_ROOT/analysis-$PLATFORM/$LOCAL_FILE
done

pushd $INDEX_ROOT
merge-analyses analysis-*/$LOCAL_FILE > analysis/$LOCAL_FILE
popd
