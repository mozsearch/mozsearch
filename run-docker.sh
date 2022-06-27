#!/bin/bash

# This command tries to start up your searchfox docker container, creating it
# from the image created in `build.docker.sh` if it doesn't already exist, and
# ensure you get a distinct bash shell in the container even if you run this
# script in multiple tabs[1].
#
# 1: We definer the container as running a canonical bash shell.  When we run
# `docker container start CONTAINER` this spins up the shell, but you don't get
# your terminal bound to the shell until we run `docker attach CONTAINER`.
# However, if we naively run `docker attach CONTAINER` in 2 separate terminals,
# you'll just end up seeing the same single underlying bash instance mirrored to
# both terminals.  This is almost certainly not what you want, so we will
# instead run an additional bash shell in the container.

# ==============================================================================

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

SHELL=/usr/bin/bash

container_exists() {
    docker container inspect ${CONTAINER_NAME} &> /dev/null
}

if container_exists; then
    CONTAINER_STATE=$(docker container inspect ${CONTAINER_NAME} | jq -r '.[0].State.Status')
    # If it's already running, run a new bash command inside the container
    if [[ $CONTAINER_STATE == "running" ]]; then
        docker exec -it ${CONTAINER_NAME} ${SHELL}
    else # start the (already created) container and attach to its canonical shell
        # this will print out our container name if we don't redirect stdout
        docker container start ${CONTAINER_NAME} > /dev/null
        docker attach ${CONTAINER_NAME}
    fi
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
        ${SHELL}
fi
