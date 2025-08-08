# syntax=docker/dockerfile:1
FROM ubuntu:24.04
ARG LOCAL_UID
ARG LOCAL_GID

COPY *.sh /infrastructure/
# this will create our user "vagrant"
RUN /infrastructure/docker-provision-pre.sh $LOCAL_UID $LOCAL_GID
USER vagrant
WORKDIR /home/vagrant

RUN /infrastructure/common-provision-pre.sh

# common-provision-pre.sh installed cargo, make sure the path is available for
# the next commands
ENV PATH=/home/vagrant/livegrep-venv/bin:/home/vagrant/.cargo/bin:$PATH

RUN /infrastructure/indexer-provision.sh
ENV PYTHONPATH=/home/vagrant/pymodules
ENV MOZSEARCH_WASM_DIR=/vagrant/scripts/web-analyze/wasm-css-analyzer/out
ENV MOZSEARCH_CLANG_PLUGIN_DIR=/vagrant/clang-plugin

# indexer-provision.sh installed Coursier, make sure the path is available for
# the next commands
ENV PATH=$PATH:/home/vagrant/.local/share/coursier/bin

RUN /infrastructure/web-server-provision.sh
RUN /infrastructure/common-provision-post.sh

# installs extra tooling only required for docker
RUN /infrastructure/docker-provision-post.sh
ENV PATH=/home/vagrant/mozsearch-firefox:$PATH

EXPOSE 16995/tcp

CMD ["/usr/bin/bash"]
