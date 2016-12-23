# Installation

There are three ways to install and run mozsearch:

* Locally on a Linux system.
* In a Vagrant container.
* On AWS.

### Local installation

Most of the configuration steps are automated in
`infrastructure/indexer-provision.sh` (specific to indexing source
code) and `infrastructure/web-server-provision` (specific to serving
the indexed code). These scripts install everything globally. You may
want to individually install some packages in a Python virtualenv or a
user-specific bin directory. Just make sure that everything is in your
`$PATH` and that the virtualenv is active when you try to index or
serve web pages.

Both scripts create an `update.sh` script that downloads and clones
the latest version of mozsearch and SpiderMonkey and builds them. You
might want to run this script once and then `git pull` and build
yourself manually. The `update.sh` script takes the release channel
and configuration repository as arguments. The release channel is
either `release` or `dev`. This determines which branch of mozsearch
will be cloned.

The configuration repository contains data about which repositories
should be indexed. For Mozilla repositories, this will be
`http://github.com/bill-mccloskey/mozsearch-mozilla`. This repository
is also cloned by `update.sh`.

### Local development

Once mozsearch is installed and the clang plugin and Rust code are
compiled, you can try to index some code.

Indexing is done almost entirely through multiple layers of shell
scripts. The layered approach allows you to control how much indexing
should be done when doing local development. This leads to a faster
edit-compile-debug cycle. As you do more development, it's a good idea
to understand how to higher-level shell scripts call the lower-level
ones and how they use arguments and environment variables to pass
data.

Indexing is largely driven by the `config.json` file in the
configuration repository ([`mozsearch-mozilla/config.json`](https://github.com/bill-mccloskey/mozsearch-mozilla/blob/master/config.json) for example). This file
lists every repo to be indexed and the paths where the index should be
stored.

The most automated way to do index is via the
`infrastructure/indexer-setup.sh` and `infrastructure/indexer-run.sh`
scripts. Here are example invocations:

```
# Assume that config/ contains a clone of
# http://github.com/bill-mccloskey/mozsearch-mozilla,
# as the update.sh script ensures.

# Note: this process will take a while, perhaps an hour or two!

mkdir index
mkdir scratch
infrastructure/indexer-setup.sh config index scratch
infrastructure/indexer-run.sh config scratch
```

The setup script will download clones of various Mozilla repositories
from Amazon S3 and update them. Then it will generate a blame
repository for them. It also creates a `config.json` file in `scratch`
that contains the full paths to each repository to index (the
`config.json` in the configuration repository has several variables
that need to be substituted).

Rather than running the setup script, you can do this work
yourself. It might make sense to do so if you already have clones
of the repositories of interest. You can also use your own
`config.json` with only the repositories of interest to you.

The `indexer-run.sh` script mostly calls `scripts/mkindex.sh`, which
does most of the work for a single repository. If you only want to
index one repository, you can call `mkindex.sh` yourself, as follows:

```
scripts/mkindex.sh config scratch/config.json mozilla-central
```

`mkindex.sh` finds the files to index, builds C++ code with the clang
plugin, analyzes JS code using SpiderMonkey's Reflect.parse
implementation, analyzes IDL files, cross-references, and outputs
HTML. Each of these steps can be done individually, although you may
have to set some environment variables by sourcing
`scripts/load-vars.sh` as `mkindex.sh` does.

Note that the `js-analyze.sh` and `output.sh` scripts allow you to
pass a regular expression filter so that they only analyze a subset of
the files in the repository.

### Local web serving

Note: Please read [the web server notes](web-server.md) before this
section.

The easiest way to test your code is by running either the Python or
Rust servers manually. Both servers take `scratch/config.json` as the
sole argument. The Python server is located at `router/router.py`. The
Rust server is at `tools/target/release/web-server`. The Rust web
server runs on port 8001 and the Python server on 8000. Both servers
try to serve up a minimal set of static files so that they can be used
without nginx.

### Vagrant

Using Vagrant is very similar to local installation. The Vagrantfile
automatically calls the provisioning scripts. From there, you can call
`update.sh` to download everything. The only advantage of Vagrant is
that you can ensure a clean system to test on.

### AWS

...todo...
