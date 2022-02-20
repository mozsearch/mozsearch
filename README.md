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

##### Ubuntu

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

A successful provisioning run will end with `mv update-log provision-update-log-2`.

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
- Run [test checks](docs/testing-checks.md) against the indexer output.
- Setup the webserver for the test repo.
- Run the webserver for the test repo.
- Run [test checks](docs/testing-checks.md) against the web server.  These are
  the same checks we ran against the indexer above plus several that are
  specific to the web-server.

After that, you can connect to http://localhost:16995/ and see Searchfox at work!

Once you've done that, you might want to read the
[Manual Indexing doc](docs/manual-indexing.md) for more details on what's
happening under the hood when you run the above make rule.


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
  from git or it can panic because of accessing beyond the end of vectors.  (The
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

## Testing changes against mozilla-central

If you are making more extensive changes to searchfox, it's usually advisable to
test them against mozilla-central before landing them.  While it's possible to
do this locally, the normal way to do this is:
- If you have made any changes to the in-tree indexing process, such as the
  clang plugin, run the relevant try jobs using the mozilla-central try
  infrastructure.  If you haven't made any changes, you can skip this step and
  the AWS indexing job will just reuse mozilla-central's most recently nightly
  searchfox data.
- Run an AWS indexing job using `trigger_indexer.py`.

Details below.

### Running mozilla-central try builds for changes to in-tree indexing

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

### Triggering an AWS indexer run

An important precondition is that you need to be a member of the "searchfox-aws"
mozillians.org group in order to have the access rights to do the following.  We
are happy to add Mozillians to this group who are actively interested in
contributing to searchfox.  Please reach out in #searchfox on
https://chat.mozilla.org/

- First, follow the [Searchfox AWS docs](docs/aws.md) to ensure you have your
  credentials working in general and that you can run the
  `infrastructure/aws/ssh.py` command and successfully get a list of active VMs.
  In particular, you will probably need to type:
  - `. env/bin/activate`
  - `eval $(maws -o awscli --profile default)`
- Push your changes to your mozsearch branch and your mozsearch-mozilla branch,
  if appropriate.
  - It's usually a good idea to explicitly make sure you've saved all your
    buffers, that `git status` shows no uncommitted changes, that
    `make build-test-repo` runs successfully, and that
    `git push -f REMOTE BRANCH` says all the commits are already there.
- Pick what "channel" you are going to use.  Generally, the right answer is the
  "dev" channel, which will display its results at https://dev.searchfox.org/
  but it's possible to use and create other channels.  You can see if anyone
  already has a server up on the "dev" channel by running the
  `infrastructure/aws/ssh.py` script.
- Pick what config file you are going to use.  Normally this is "config1.json"
  which includes the mozilla-central repo and a few other repositories that
  don't have all the bells and whistles turned on.  You can use a different
  config file or edit config1.json to not contain repositories you aren't
  interested in to make things go faster.
  - You don't have to have your own branch of mozsearch-mozilla!  You can use
    https://github.com/mozsearch/mozsearch-mozilla and it's fine that it doesn't
    have a branch with the name of your development branch.  The scripts will
    automatically fall back to the default branch.
- If you didn't run a custom m-c try job, you can edit the following:
```
infrastructure/aws/trigger_indexer.py \
  https://github.com/some-user/mozsearch \
  https://github.com/some-user/mozsearch-mozilla \
  config1.json \
  some-development-branch \
  dev
```
- If you did run a custom m-c try job, the only difference is the addition of a
  `--setenv TRYPUSH_REV=full-40char-hash` to to the command.  Using a truncated
  hash won't work because, unlike the hg/git command line, the taskcluster
  server can't/won't expand revisions and searchfox doesn't apply any transforms
  at this time.  So this looks like:
```
infrastructure/aws/trigger_indexer.py \
  --setenv TRYPUSH_REV=full-40char-hash \
  https://github.com/some-user/mozsearch \
  https://github.com/some-user/mozsearch-mozilla \
  config1.json \
  some-development-branch \
  dev
```
- The author of the HEAD commit of the mozsearch branch that gets checked out
  will receive an email when indexing completes or when it fails.  This means
  that if you are testing changes that you've only made to mozsearch-mozilla,
  you will likely need to create a silly change to the mozsearch repo.
  - If your indexing run failed, the indexer will move the current state of its
    scratch SSD to the durable S3 storage and then stop itself.  The `ssh.py`
    command will restart the indexer for you when you connect to it.  Then you
    will need to investigate what went wrong.  See the section on Debugging
    errors in our [AWS docs](docs/aws.md) for more info.
  - If your indexing run succeeded, that means the indexer successfully kicked
    off a web-server.  You should be able to connect to the searchfox UI at
    https://dev.searchfox.org/ or whatever the name of the channel you used was.
    You should also be able to use `infrastructure/aws/ssh.py` to connect to the
    web-server and explore the contents of the built index under `~/index`.
- When you are done with any of the above severs, you can use
  `infrastructure/aws/terminate-indexer.py` to destroy the VM which will also
  clean up any S3 storage the index used.  You can find which servers are yours
  via the `ssh.py` script, making sure to pay attention to the "channel" tag;
  you don't want to terminate any of the release servers!

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
* [Testing](docs/testing-checks.md)
* [Web serving](docs/web-server.md)
* [Deploying to AWS](docs/aws.md)
* [Adding new repos](docs/newrepo.md)
* [Bash scripting cheatsheet](docs/bash-scripting-cheatsheet.md)
