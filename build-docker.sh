#!/bin/bash

docker build \
    -t searchfox \
    --build-arg LOCAL_UID=$(id -u $USER) \
    --build-arg LOCAL_GID=$(id -g $USER) \
    infrastructure/
