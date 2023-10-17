# syntax=docker/dockerfile:1
FROM ubuntu:22.04
ARG LOCAL_UID
ARG LOCAL_GID

COPY *.sh /infrastructure/
# this will create our user "vagrant"
RUN /infrastructure/docker-provision.sh $LOCAL_UID $LOCAL_GID
USER vagrant
WORKDIR /home/vagrant

RUN /infrastructure/common-provision-pre.sh

# common-provision-pre.sh installed cargo, make sure the path is available for
# the next commands
ENV PATH=/home/vagrant/.cargo/bin:$PATH

RUN /infrastructure/indexer-provision.sh

# indexer-provision.sh installed Coursier, make sure the path is available for
# the next commands
ENV PATH=$PATH:/home/vagrant/.local/share/coursier/bin

RUN /infrastructure/web-server-provision.sh
RUN /infrastructure/common-provision-post.sh

EXPOSE 80/tcp

CMD ["/usr/bin/bash"]
