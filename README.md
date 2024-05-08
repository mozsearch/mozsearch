# Mozsearch

Mozsearch is the backend for the [Searchfox](https://searchfox.org)
code indexing tool. Searchfox runs inside AWS, but you can develop on
Searchfox locally using Vagrant.

## Docker Setup For Local Development

We've moved from using Vagrant to docker for development because it's comparably
painless and because, especially on linux, it's great for the searchfox container
to be able to use all your CPU cores and only use memory when it needs it
(instead of all the time).

That said, if you want to use something else, that's fine.  If you look at our
docker scripts, they're just a bunch of shell scripts we run under Ubuntu and
they're basically the same as they were under Vagrant.  And we also just run
those scripts on top of a basic Ubuntu AMI for provisioning our instances.  If
you can run Ubuntu under a VM, you can run Searchfox in there.

Could you run searchfox outside of a container/VM?  Probably?  But, dependencies
are a hassle.

### Install docker.io

Can you already run "docker" on your command line and have it work?  Then you
can skip this section!

#### Important Docker Licensing Notes

Docker has changed their licensing of their "Docker Desktop" packages.  We do
not recommend using Docker Desktop; you don't need it as long as you already
are able to run Ubuntu directly or under (para)virtualization like WSL2.

If you do want to use it and you work at Mozilla, you should acquire a license
through Service Desk before doing anything else.  If you already have a license,
that's fine too.

#### Alternative to Docker: Podman

Podman can be used almost as a drop-in replacement to Docker. Just make sure
that:
- You have a podman wrapper or symlink named docker in your PATH, the scripts
  call docker extensively.
- You set the environment variable PODMAN_USERNS="keep-id" − or the equavalent
  option in containers.conf. The source repository is bind-mounted inside the
  container and the user in the container gets the same UID/GID as the caller.
  This makes sure that the vagrant user in the container can read/write inside
  the bind mount at /vagrant.

> Note for Nix users: the devShell in flake.nix provides both of those requirements.

#### Installing on macOS/OS X

I don't think anyone has tried this yet, but it seems like there are a variety
of options.  Here are some I've just briefly researched:
- Use podman.  https://podman.io/docs/installation explains how to use QEMU on
  macOS.
- Use Colima: https://github.com/abiosoft/colima
- Use lima, the thing that colima wraps: https://github.com/lima-vm/lima

If you use a thing and it works, please let us know and we can update these
docs.  Or better yet, you can submit a pull request for this doc!

#### Installing on WSL2 on Windows 10 or Windows 11

First, you need Ubuntu installed under WSL2.  If you already have a sufficiently
up-to-date version (I would suggest Ubuntu 22.04 or later), you can skip this
next step.

From the windows store, install "Ubuntu 22.04.1 LTS" or whatever updated version
there is.  Then I launched the "Terminal" app (which you may need to also
install from the Windows store).  I used the drop-down arrow to the right of the
add a tab "+" button and picked "Ubuntu 22.04.1 LTS".  This launched an
installer that ran for ~30 seconds and then prompted me to be okay to switch
back to the terminal.  Specific steps:
- I picked my language (“English”)
- I picked my username and entered a user password
- On the “WSL configuration options” I left things at their default (“/mnt/” as the mount
  location, empty mount option, and “enable host generation” and “enable resolv.conf”
  generation checked.)
- I chose the reboot option.
- After the reboot, the terminal showed a terminal logged in as my user.

Now let's keep going through the next sections.

#### Installing docker.io on WSL2 and Linux

Run the following commands in your normal Linux terminal on Linux, and under
WSL2 on Windows, use the Terminal app with your "Ubuntu 22.04.1 LTS" tab open.

- `sudo apt update` to update the package registry, you may need to enter your
  password because you’re using sudo.
- `sudo apt install -y docker.io`
- `sudo adduser $USER docker` to add yourself to the docker group.
- Unfortunately you won’t effectively be in the group until you are running
  under a fresh login.  There are 2 easy ways to do this and 1 annoying way:
  1. If you are using windows terminal, you can close the existing tab and open
     a new tab.  In the new tab, you should then see “docker” when you run
     `groups`.
  2. Run `su - $USER` and this will dump you in a freshly logged in shell where
     you should see “docker” when you run `groups`.
  3. Logout and login again.
- Under WSL2: `sudo update-alternatives --config iptables` and pick the
  “iptables-legacy” option, probably by hitting 1 and hitting enter.  This
  switches iptables to legacy mode.  This was necessary to make docker.io happy
  under WSL2 for me.

#### Installing deps that will make VS Code happy

VS Code is a very nice editor and it hooks up quite nicely to rust-analyzer via
the language server protocol (LSP).  Rust-analyzer needs to run somewhere and
VS code's remote support is quite excellent.  I personally do the following on
these platforms:
- On Windows, I install the following dependencies in the "Ubuntu 22.04.1 LTS"
  install and then connect to it via VS code's support for connecting to WSL(2)
  instances.  This results in rust-analyzer running in the WSL2 instance but not
  inside the docker instance.  (I've never tried having it run inside docker;
  that might not be a bad idea?)
- On linux I just run VS code locally with no remote connections which means
  rust-analyzer is running locally (and outside of the docker instance).  Having
  the following things installed is still useful.

Installation steps:
- Go to https://rustup.rs/ and copy and paste its command into your terminal.
  What could go wrong?
- `sudo apt install -y clang pkg-config libssl-dev cmake`

### Check out mozsearch

- Change into whatever directory you want to keep your “mozsearch” checkout in.
- `git clone https://github.com/mozsearch/mozsearch.git` to check out searchfox
  if you haven’t already
- `git submodule update --init --recursive` to check out the relevant submodules

### Now you can build and run the mozsearch docker container!

- Run `./build-docker.sh` to provision and setup the “VM” / container.  This
  will run our shell scripts to create multiple layers of filesystem.  Note that
  the "livegrep" build phase may seem to hang for several minutes at one point
  when it's looking for boost dependencies and this is unfortunately expected.
  Don't ctrl-c out (although it's harmless if you do, you can re-run the command
  later).
- Run `./run-docker.sh` to start up the VM/container.  It will shutdown when you
  exit from the shell so leave the shell running to keep the web-server
  accessible.
- If you want more shells inside the VM, you can invoke `./run-docker.sh` from
  other terminal tabs and they will connect to that same running container.
  Note that if you log out of the first invocation that actually started the
  container, it will close all the other terminals.

Those scripts have default container, image, and volume names ("searchfox",
"searchfox", "searchfox-vol") that can be overridden.  You would usually only do
this if you want multiple orthogonal searchfox checkouts on the same machine.
(But do note that currently the script does not make it possible to override the
decision to use port 16995, so you can't run both containers at the same time.)

## Instant Fun with the Test Repo

Once you are able to successfully `./run-docker.sh`, once you are inside at that
shell, you can do the following:

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
  - `aws sso login` (this assumes your "env/bin/activate" script above sets the
    `AWS_PROFILE` env variable; in our docs referenced above we added that to
    the activate script).
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
- Once the indexer starts, [it generates a log file in `~/index-log`](infrastructure/aws/main.sh) on the instance.  You can track the progress by logging into the instance with `ssh.py` and running `tail -f ~/index-log`.  [The log file is copied to `/index/index-log`](infrastructure/aws/index.sh) once the indexing completes.
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

* Production Reference
  * [Adding new repos](docs/newrepo.md)
  * [AWS Production Infrastructure Details and Runbooks](docs/aws.md)
* Implementation Documentation
  * [Analysis](docs/analysis.md)
  * [Blame caching](docs/blame.md)
  * [Cross-referencing](docs/crossref.md)
  * [Index Directory Contents](docs/index-directory-contents.md)
  * [HTML output](docs/output.md)
  * [Testing](docs/testing-checks.md)
  * [Web serving](docs/web-server.md)
* Cheatsheets
  * [Bash scripting cheatsheet](docs/bash-scripting-cheatsheet.md)
  * [Liquid templating cheatsheet](docs/liquid-templating-cheatsheet.md)
  * [searchfox-tool cookbook](docs/searchfox-tool-cookbook.md)
