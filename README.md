# Mozsearch

Mozsearch is the backend for the [Searchfox](https://searchfox.org)
code indexing tool. Searchfox runs inside AWS, but you can develop on
Searchfox locally using Vagrant.

## Vagrant setup for local development

### Setting up the VM

We use Vagrant to setup a virtual machine.  This may be the most frustrating part of
working with Searchfox.  If you can help provide better/more explicit instructions
for your platform, please do!

#### Linux

Important note: In order to expose the Searchfox source directory into the VM, we
need to be able to export it via NFS.  If you are using a FUSE-style filesystem
like `eCryptFS` which is a means of encrypting your home directory, things will not
work.  You will need to move searchfox to a partition that's a normal block device
(which includes LUKS-style encrypted partitions, etc.)

##### Ubuntu 19.10

```shell
# make sure the apt package database is up-to-date
sudo apt update
# vagrant will also install vagrant-libvirt which is the vagrant provider we use.
# virt-manager is a UI that helps inspect that your VM got created
# The rest are related to enabling libvirt and KVM-based virtualization
sudo apt install vagrant virt-manager qemu libvirt-daemon-system libvirt-clients

git clone https://github.com/mozsearch/mozsearch
cd mozsearch
git submodule update --init
vagrant up
```
##### Other Linux
Note: VirtualBox is an option on linux, but not recommended.

1. [install Vagrant](https://www.vagrantup.com/downloads.html).
2. Install libvirt via [vagrant-libvirt](https://github.com/vagrant-libvirt/vagrant-libvirt).
   Follow the [installation instructions](https://github.com/vagrant-libvirt/vagrant-libvirt#installation).
  - Note that if you didn't already have libvirt installed, then a new `libvirt`
    group may just have been created and your existing logins won't have the
    permissions necessary to talk to the management socket.  If you do
    `exec su -l $USER` you can get access to your newly assigned group.
  - See troubleshooting below if you have problems.
  
Once that's installed:
```shell
git clone https://github.com/mozsearch/mozsearch
cd mozsearch
git submodule update --init
vagrant up
```

If vagrant up times out in the "Mounting NFS shared folders..." step, chances
are that you cannot access nfs from the virtual machine.

Under stock Fedora 31, you probably need to allow libvirt to access nfs:

```
firewall-cmd --permanent --add-service=nfs --zone=libvirt
firewall-cmd --permanent --add-service=rpc-bind --zone=libvirt
firewall-cmd --permanent --add-service=mountd --zone=libvirt
firewall-cmd --reload
```
  
#### OS X and Windows
Note: The current Homebrew version of Vagrant is currently not able to use the most
recent version of VirtualBox so it's recommended to install things directly via their
installers.

1. [install Vagrant](https://www.vagrantup.com/downloads.html).
2. Visit the [VirtualBox downloads page](https://www.virtualbox.org/wiki/Downloads) and
    follow the instructions for your OS.
 
Then clone Mozsearch and provision a Vagrant instance:
```
git clone https://github.com/mozsearch/mozsearch
cd mozsearch
git submodule update --init

vagrant plugin install vagrant-vbguest
vagrant up
```

### Once vagrant up has started...

The last step will take some time (10 or 15 minutes on a fast laptop)
to download a lot of dependencies and build some tools locally.  **Note
that this step can fail!**  Say, if you're at a Mozilla All-Hands and the
network isn't exceedingly reliable.  In particular, if you are seeing
errors related to host resolution and you have access to a VPN, it may
be advisable to connect to the VPN.

A successful provisioning run will end with `default: + chmod +x update.sh`.

In the event of failure you will want to run
`vagrant destroy` to completely delete the VM and then
run `vagrant up` again to re-create it.  The base image gets cached on
your system, so you'll save ~1GB of download, but all the Ubuntu package
installation will be re-done.

After `vagrant up` completes, ssh into the VM as follows. From this point
onward, all commands should be executed inside the VM.

```
vagrant ssh
```

At this point, your Mozsearch git directory has been mounted into a
shared folder at `/vagrant` in the VM. Any changes made from inside or
outside the VM will be mirrored to the other side. Generally I find it
best to edit code outside the VM, but any commands to build or run
scripts must run inside the VM.

## Instant Fun with the Test Repo

```
cd /vagrant
make build-test-repo
```

The above process will:
- Build necessary tools.
- Setup the indexer for the test repo.
- Run the indexer for the test repo.
- Setup the webserver for the test repo.
- Run the webserver for the test repo.

After that, you can connect to http://localhost:16995/ and see Searchfox at work!

Once you've done that, you might want to read the next section to understand
what was happening under the hood.

## Manual Labor with the Test Repo

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
/vagrant/infrastructure/web-server-setup.sh /vagrant/tests config.json ~/index ~

# Starts the Python and Rust servers needed for Mozsearch.
/vagrant/infrastructure/web-server-run.sh /vagrant/tests ~/index ~
```

At this point, you should be able to visit the server, which is
running on port 80 inside the VM and port 16995 outside the VM. Visit
`http://localhost:16995/` to do so.

### Testing locally with blame using the "searchfox" test config

The `tests` configuration defined at `tests/config.json` is very helpful, but it
isn't configured to use a blame repository or generate blame UI output.  If
you're making changes that affect the blame UI or might interact with it, it
helps to test with it!

The `searchfox` configuration defined at `tests/searchfox-config.json` exists
for this purpose.  It indexes the entirety of the repository.  It can be built
via the `Makefile` by invoking the following to build the index at
`~/searchfox-index` (whereas `tests` is built at `~/index`).

```
make build-searchfox-repo
```

Note that you will need to do a couple things for this to work right:
- You need to make sure any changes you've made to the searchfox repository are
  committed to git.  `output-file.rs` depends on the blame repository having
  lines that match up exactly with the state of the source files checked out
  from git or it can panic because of accesing beyond the end of vectors.  (The
  blame data will also be wrong.)
- You need to make sure the blame repository has been updated.  The Makefile
  will take care of this for you, but if you're running `indexer-run.sh`
  manually without first running `indexer-setup.sh`, you may experience
  problems.

Also note that this will terminate any previously running `tests` web servers
even though the indexes live at different directories (`~/index` versus
`~/searchfox-index`). If you find that you want both the `tests` and `searchfox`
configurations to be served at the same time, you can add a new configuration
file and update these docs and submit a pull requests.  Thanks in advance!

## Indexing Mozilla code locally

Although it can take a long time, it's sometimes necessary to index
the Mozilla codebase to test changes to Searchfox. How to do that
depends on what you want to test.
If you are making changes to the clang-plugin, you need to do these steps first.
If not, you can skip to the next set of steps in this section.

### Testing clang-plugin changes

For testing changes to the clang-plugin, run these steps, followed by the
steps in the next section.

* Make your changes to the build/clang-plugin/mozsearch-plugin/ folder
  in mozilla-central, and push them to try. Ensure that your try push has
  all the searchfox jobs as well as the bugzilla-components job. The following
  try syntax will accomplish this:
```
./mach try fuzzy --full -q "'searchfox" -q "'bugzilla-component"
```
* Record the full hg revision hash (40 characters) of the try push.
* In the vagrant instance, run the following command in `/vagrant/`:
```
TRYPUSH_REV=<40-char-rev-hash> make trypush
```
This will clone the Mozilla configuration into ~/mozilla-config, and
generate a reduced config that has just the mozilla-central tree, but
use the code and artifacts from your try push when building the index.
It will build the index into a `~/trypush-index` folder to keep it separate
from any `~/mozilla-index` folders you might have lying around.
It's very similar to the operations described in the next section
which will build an index using the latest mozilla-central version with
searchfox artifacts.

### Testing basic changes

Note: You can also just do `make build-mozilla-repo` in `/vagrant` to have it
idempotently do the following for you.

```
# Clone the Mozilla configuration into ~/mozilla-config, if you haven't
# already done so. (If you are testing clang-plugin changes, you will
# already have done this and made modifications to mozilla-central/setup,
# so no need to clone again).
git clone https://github.com/mozsearch/mozsearch-mozilla ~/mozilla-config

# Manually edit the ~/mozilla-config/config.json to remove trees you don't
# care about (probably NSS and comm-central). Make sure to remove any trailing
# commas if they're not valid JSON!
nano ~/mozilla-config/config.json

# Make a new index directory.
mkdir ~/mozilla-index

# This step will download copies of the Mozilla code and blame information,
# along with the latest taskcluster artifacts, so it may be slow.
/vagrant/infrastructure/indexer-setup.sh ~/mozilla-config config.json ~/mozilla-index

# This step involves unpacking the taskcluster artifacts, and indexing a lot of
# code, so it will be slow!
/vagrant/infrastructure/indexer-run.sh ~/mozilla-config ~/mozilla-index
```

Note: By default, `indexer-setup.sh` keeps the contents of the working
directory (in the example above, that's `~/mozilla-index`). In case you want
to delete the contents of the working directory, define CLEAN_WORKING=1
when calling `indexer-setup.sh`.

### Locally indexing a try push

If you are not hacking on Searchfox itself, but just want to build a local
index of changes to mozilla-central (e.g. you are reviewing a complex
patchset, and want to have a Searchfox instance with those patches applied)
follow the same steps as described in the "Testing clang-plugin changes"
section above, except obviously you don't need to make any changes to
the clang-plugin, but just include the patches you care about in the try
push.

## Background on Mozsearch indexing

The Mozsearch indexing process has three main steps, depicted here:

![Indexing diagram](/docs/indexing.png?raw=true)

Here are these steps in more detail:

* A language-specific analysis step. This step processes C++, Rust,
  JavaScript, and IDL files. For each input file, it generates a
  line-delimited JSON file as output. Each line of the output file
  corresponds to an identifier in the input file. The line contains a
  JSON object describing the identifier (the symbol that it refers to,
  whether it's a use or a def, etc.). More information on the analysis
  format can be found in the [analysis
  documentation](docs/analysis.md).

* Full-text index generation. This step generates a single large index
  file, `livegrep.idx`. This self-contained file can be used to do
  regular expression searches on every text file in the input. The
  index is generated by the `codesearch` tool, which is part of
  [Livegrep](https://github.com/livegrep/livegrep). The same
  `codesearch` tool is used by the web server to search the index.

* Blame generation. This step takes a git repository as input and
  generates a "blame repository" as output. Every revision in the
  original repository has a corresponding blame revision. The blame
  version of the file will have one line for every line in the
  original file. This line will contain the revision ID of the
  revision in the original repository that introduced that line. This
  format makes it very fast to look up the blame for an arbitrary line
  at an arbitrary revision. More information is available on
  [blame caching](docs/blame.md).

Once all these intermediate files have been generated, a
cross-referencing step merges all of the symbol information into a set
of summary files: `crossref`, `jumps`, and `identifiers`. These files
are used for answering symbol lookup queries in the web server and for
generating static HTML pages. More detail is available on
[cross-referencing](docs/crossref.md).

After all the steps above, Mozsearch generates one static HTML file
for every source file. These static HTML pages are served in response
to URLs like
`https://searchfox.org/mozilla-central/source/dir/foobar.cpp`. Most
requests are for URLs of this type. Generating the HTML statically
makes it very quick for the web server frontend (nginx) to serve these
requests.

HTML generation takes as input the analysis JSON. It uses this data to
syntax highlight the code more effectively (so that it can color types
differently from variables, and definitions differently from uses). It
also uses the analysis JSON, as well as the `jumps` file, to generate
the context menu information for each identifier. In addition, the
blame repository is used to generate HTML for the blame strip.

## More background

* [Index Directory Contents](docs/index-directory-contents.md)
* [Blame caching](docs/blame.md)
* [Analysis](docs/analysis.md)
* [Cross-referencing](docs/crossref.md)
* [HTML output](docs/output.md)
* [Web serving](docs/web-server.md)
* [Deploying to AWS](docs/aws.md)
* [Adding new repos](docs/newrepo.md)
* [Bash scripting cheatsheet](docs/bash-scripting-cheatsheet.md)
