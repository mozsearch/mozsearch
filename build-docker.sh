#!/usr/bin/env bash

# See `run-docker.sh` for context; it uses these defs too. Yes, we could source.
IMAGE_NAME=${SEARCHFOX_DOCKER_IMAGE_NAME:-searchfox}
CONTAINER_NAME=${SEARCHFOX_DOCKER_CONTAINER_NAME:-searchfox}
VOLUME_NAME=${SEARCHFOX_DOCKER_VOLUME_NAME:-searchfox-vol}

docker build \
    -t ${IMAGE_NAME} \
    --build-arg LOCAL_UID=$(id -u $USER) \
    --build-arg LOCAL_GID=$(id -g $USER) \
    infrastructure/

## Clean up any existing container and affiliated volumes
#
# Because our provisioning process installs a lot of things into our user's home
# dir ("vagrant" for legacy reasons), it's necessary for us to remove the volume
# we create in addition to any existing container.  Because volumes can only be
# removed after their container, this mandates removing the container first.  We
# want to remove the container anyways since we want the container to use our
# freshly updated image!

container_exists() {
    docker container inspect ${CONTAINER_NAME} &> /dev/null
}

volume_exists() {
    docker volume inspect ${VOLUME_NAME} &> /dev/null
}

if container_exists; then
    echo "Removing existing container: ${CONTAINER_NAME}"
    docker rm ${CONTAINER_NAME}
fi


if volume_exists; then
    # Note: There's a force flag but I'm not sure what it's for since it doesn't do
    # anything if the container exists.
    echo "Removing existing volume: ${VOLUME_NAME}"
    docker volume rm ${VOLUME_NAME}
fi

# You don't have to actually create the volume, but this is the closest thing to
# what we did for Vagrant and it seems potentially desirable.  The run-docker.sh
# script should handle if this does not exist.
#
# If the volume is not created, then any changes to the user's home directory
# will be lost when the first `run-docker.sh` invocation that started the
# container ends.  Since this is where our indexing byproducts exist, it can be
# nice for this directory to be durable.  And in the case of develoeprs using
# `make build-mozilla-repo` it's particularly desirable because of how long the
# build process takes.
docker volume create ${VOLUME_NAME}

