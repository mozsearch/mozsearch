#!/bin/bash

CONTAINER_NAME=searchfox
IMAGE_NAME=searchfox
THIS_DIR=$(pwd)
# For consistency we mount the source dir at /vagrant still
INSIDE_CONTAINER_DIR=/vagrant

# connect to this port on your computer
OUTSIDE_CONTAINER_PORT=16995
# which is served at this port inside the container
INSIDE_CONTAINER_PORT=80

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
    $IMAGE_NAME \
    /usr/bin/bash || docker container start searchfox && docker attach searchfox
