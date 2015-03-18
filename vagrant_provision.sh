#!/bin/sh
# Shell script to provision the vagrant box
#
# This is idempotent, even though I'm not sure the shell provisioner requires
# it to be.

set -e
set -x

apt-get update
# clean out redundant packages from vagrant base image
apt-get autoremove -y

apt-get install -y nginx
mkdir -p /etc/nginx/sites-enabled
cat >/etc/nginx/sites-enabled/mozsearch.conf <<THEEND
server {
  listen 8000 default_server;
  sendfile off;

  location /static {
    root /home/vagrant/mozsearch;
  }

  location = /router {
    fastcgi_pass 127.0.0.1:8888;
    include fastcgi_params;
  }

  location / {
    rewrite ^(.*)$ /router?\$1 last;
    break;
  }
}
THEEND
chmod 0644 /etc/nginx/sites-enabled/mozsearch.conf

# mercurial
apt-get install -y mercurial

# DXR itself:
# pkg-config is so (trilite's?) make clean works.
apt-get install -y git llvm-3.5 libclang-3.5-dev clang-3.5 pkg-config
# --force overrides any older-version LLVM alternative lying around, letting
# us upgrade by provisioning rather than destroying the whole box:
update-alternatives --force --install /usr/local/bin/llvm-config llvm-config /usr/bin/llvm-config-3.5 0
# There is no clang++ until we do this:
update-alternatives --force --install /usr/local/bin/clang++ clang++ /usr/bin/clang++-3.5 0

# Firefox build prerequisites
# From https://developer.mozilla.org/en-US/docs/Mozilla/Developer_guide/Build_Instructions/Linux_Prerequisites
apt-get install zip unzip mercurial g++ make autoconf2.13 yasm libgtk2.0-dev libglib2.0-dev libdbus-1-dev libdbus-glib-1-dev libasound2-dev libcurl4-openssl-dev libiw-dev libxt-dev mesa-common-dev libgstreamer0.10-dev libgstreamer-plugins-base0.10-dev libpulse-dev m4 flex ccache
