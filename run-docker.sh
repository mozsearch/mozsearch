#!/bin/bash

# If you have multiple searchfox checkouts on your machine and you want to be
# able to have them operate in isolation, you can define the variables below.
# I would suggest adding them to the appropriate env/bin/activate script that
# you use; `activate` is for bash but there are different suffixed files for
# other shell types.
CONTAINER_NAME=${SEARCHFOX_DOCKER_CONTAINER_NAME:-searchfox}
IMAGE_NAME=${SEARCHFOX_DOCKER_IMAGE_NAME:-searchfox}

THIS_DIR=$(pwd)
# For consistency we mount the source dir at /vagrant still
INSIDE_CONTAINER_DIR=/vagrant

# connect to this port on your computer
OUTSIDE_CONTAINER_PORT=16995
# which is served at this port inside the container
INSIDE_CONTAINER_PORT=80

container_exists() {
    docker container inspect ${CONTAINER_NAME} &> /dev/null
}

if container_exists; then
    # this will print out our container name if we don't redirect stdout
    docker container start ${CONTAINER_NAME} > /dev/null
    docker attach ${CONTAINER_NAME}
else
    # flags:
    # - `-it`: `i` is interactive, `t` is allocate a pseudo-tty
    # - `--name`: controls the name that is used to refer to the container for other
    #   commands.  For example, `docker container stop $NAME` and
    #   `docker container rm $NAME`.
    # - `--mount`: lets us bind the current directory into the container.
    # - `-p`: specifies the port mapping to expose the nginx web-server (when
    #   running; it doesn't automatically run!) on localhost port 16995.
    docker run \
        -it \
        --name $CONTAINER_NAME \
        --mount type=bind,source=${THIS_DIR},target=${INSIDE_CONTAINER_DIR} \
        -p ${OUTSIDE_CONTAINER_PORT}:${INSIDE_CONTAINER_PORT} \
        ${IMAGE_NAME} \
        /usr/bin/bash
fi
