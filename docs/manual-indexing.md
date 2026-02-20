## Manually Running Test Repo Tests

### Build Necessary Tools

The first step is to build all the statically compiled parts of
Mozsearch:

```
# This clang plugin analyzes C++ code and is written in C++.
cd /vagrant/clang-plugin
make

# The Rust code is stored here. We do a release build since our scripts
# look in tools/target/release to find binaries.
cd /vagrant/tools
cargo build --release
```

### Testing locally using the "tests" repository

Mozsearch chooses what to index using a set of configuration
files. There is a test configuration inside the Mozsearch `tests`
directory. We'll use this configuration for testing. However, Mozilla
code indexing is done using the
[mozsearch-mozilla](https://github.com/mozsearch/mozsearch-mozilla)
repository.

The `config.json` file is the most important part of the
configuration. It contains metadata about the trees to be indexed. For
example, it describes where the files are stored, whether there is a
git repository that backs the files to be indexed, and whether there
is blame information available.

Mozsearch stores all the indexed information in a directory called the
index. This directory contains a full-text search index, a map from
symbol names to where they appear, a list of all files, and symbol
information for each file.

The first step in indexing is to run the `indexer-setup.sh`
script. This script sets up the directory structure for the index. In
some cases, it will also download the repositories that will be
indexed. In the case of the test repository, though, all the files are
already available. From the VM, run the following command to create
the index directory at `~/index`.

```
mkdir ~/index
/vagrant/infrastructure/indexer-setup.sh /vagrant/tests config.json ~/index
```

Now it's time to index! To do that, run the `indexer-run.sh`
script. It will compile and index all the C++ and Rust files and
also do whatever indexing is needed on JS, IDL, and IPDL files.

```
/vagrant/infrastructure/indexer-run.sh /vagrant/tests ~/index
```

Now is a good time to look through the `~/index/tests` directory to
look at all the index files that were generated. To begin serving web
requests, we can start the server as follows:

```
# Creates a configuration file for nginx. The last path gives the location
# where log files are stored.
/vagrant/infrastructure/web-server-setup.sh /vagrant/tests config.json ~/index ~ ~

# Starts the Python and Rust servers needed for Mozsearch.
/vagrant/infrastructure/web-server-run.sh /vagrant/tests ~/index ~ ~ NO_CHANNEL NO_EMAIL
```

At this point, you should be able to visit the server, which is
running on port 80 inside the VM and port 16995 outside the VM. Visit
`http://localhost:16995/` to do so.

## Indexing Mozilla code locally

DOCUMENTATION DISCLAIMER: This content was extracted from
[the primary README](../README.md) because it's not something you're likely to
want to do and the searchfox developers haven't done the below in a while.
Please see the section "Testing changes against firefox-main" there instead.
If you really want to run things locally, be aware that the current default VM
configuration is probably far too limiting and you will want to change it
locally so that the VM can use more of your processor cores, more RAM, more
disk, etc.

Although it can take a long time, it's sometimes necessary to index
the Mozilla codebase to test changes to Searchfox. How to do that
depends on what you want to test.

If you are making changes to the clang-plugin, you should first follow the steps
to run a try build from the primary readme.

### Clone config repository

You first need to clone the config repository into the mozsearch repository:

```
cd path_to_mozsearch
git clone https://github.com/mozsearch/mozsearch-mozilla config
```

### Testing basic changes

Note: You can also just do `make build-firefox-repo` in `/vagrant` to have it
idempotently do the following for you.

```
# Make a new index directory.
mkdir ~/firefox-index

# This step will download copies of the Mozilla code and blame information,
# along with the latest taskcluster artifacts, so it may be slow.
/vagrant/infrastructure/indexer-setup.sh /vagrant/config just-fm.json ~/firefox-index

# This step involves unpacking the taskcluster artifacts, and indexing a lot of
# code, so it will be slow!
/vagrant/infrastructure/indexer-run.sh /vagrant/config ~/firefox-index
```

Note: By default, `indexer-setup.sh` keeps the contents of the working
directory (in the example above, that's `~/firefox-index`). In case you want
to delete the contents of the working directory, define CLEAN_WORKING=1
when calling `indexer-setup.sh`.

### Indexing m-c try builds locally

See [the main README](../README.md) section on how to run a try job.  Once it's
completed, make note of the hg revision and then continue with the following:

* In the vagrant instance, run the following command in `/vagrant/`:
```
TRYPUSH_REV=<40-char-rev-hash> make trypush
```
This uses `just-fm.json` config, but use the code and artifacts from your try
push when building the index.
It will build the index into a `~/trypush-index` folder to keep it separate
from any `~/firefox-index` folders you might have lying around.
It's very similar to the operations described in the next section
which will build an index using the latest firefox-main version with
searchfox artifacts.

### Locally indexing a try push

If you are not hacking on Searchfox itself, but just want to build a local
index of changes to firefox-main (e.g. you are reviewing a complex
patchset, and want to have a Searchfox instance with those patches applied)
follow the same steps as described in the "Testing clang-plugin changes"
section above, except obviously you don't need to make any changes to
the clang-plugin, but just include the patches you care about in the try
push.
