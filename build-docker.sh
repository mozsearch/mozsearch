#!/bin/bash

# See `run-docker.sh` for context.
IMAGE_NAME=${SEARCHFOX_DOCKER_IMAGE_NAME:-searchfox}

docker build \
    -t ${IMAGE_NAME} \
    --build-arg LOCAL_UID=$(id -u $USER) \
    --build-arg LOCAL_GID=$(id -g $USER) \
    infrastructure/
