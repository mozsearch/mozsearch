# syntax=docker/dockerfile:1
FROM ubuntu:24.04
ARG LOCAL_UID
ARG LOCAL_GID

# this will create our user "vagrant"
COPY *.sh /infrastructure/
RUN /infrastructure/docker-provision.sh $LOCAL_UID $LOCAL_GID
USER vagrant
WORKDIR /home/vagrant
ENV USER=vagrant

# install Nix
RUN sudo apt install -y xz-utils
RUN curl -L https://nixos.org/nix/install | sh -s -- --no-daemon --yes
RUN mkdir -p "$HOME/.config/nix" && echo "experimental-features = nix-command flakes" >> "$HOME/.config/nix/nix.conf"
RUN echo "source $HOME/.nix-profile/etc/profile.d/nix.sh" >> "$HOME/.bashrc"

RUN sudo apt install -y make git

EXPOSE 16995/tcp

CMD ["/usr/bin/bash"]
