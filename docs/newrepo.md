# Adding a new repository

The basic steps needed to add a new repository to searchfox are listed below. This assumes as a prerequisite
that you have set up the Vagrant VM as documented in the [top level README](../README.md).

## 1. Create a tarball with the git repo

This is as simple as running `git clone` to clone the repo locally, and then running `tar` to make a tarball.
Note that it's best to do this inside your mozsearch folder so that it's automatically mirrored inside the
vagrant VM instance for other steps below. For example:

```
cd $MOZSEARCH
git clone https://github.com/mozilla/glean git
tar cf glean.tar git
```

If you're cloning a hg repo, use cinnabar:

```
git clone -b branches/default/tip hg::https://hg.mozilla.org/hgcustom/version-control-tools git
pushd git
git branch -m master
git config fetch.prune true
popd
tar cf version-control-tools.tar git
```

## 2. Create a tarball of the blame repo

To create the blame repo, you can manually run the `build-blame` tool. If your git repo has hg metadata that
git-cinnabar can access, that will also be included into the blame repo. If you don't have git-cinnabar
installed at all, set `CINNABAR=0` in your environment before running the `build-blame` tool. You can run
this step inside or outside the Vagrant VM, wherever you prefer. The instructions assume you're inside
the VM because that's usually where the rust code is built and run.

```
cd /vagrant
pushd tools && cargo +nightly build --release && popd
mkdir blame
pushd blame && git init . && popd
tools/target/release/build-blame ./git ./blame # this might take a while, depending on your repo size
tar cf glean-blame.tar blame
```

## 3. Upload the two tarballs to the S3 bucket

This uses the `upload.py` script, and assumes you have appropriate AWS permissions (see the section on
[Setting up AWS access locally](aws.md#setting-up-aws-locally)). These commands should be run in whatever
environment you have AWS access with (typically outside the vagrant VM). If you do not have AWS access,
please contact one of the searchfox maintainers who can do the upload for you so you can finish debugging
and testing the setup.

```
cd $MOZSEARCH
infrastructure/aws/upload.py ./glean.tar searchfox.repositories glean.tar
infrastructure/aws/upload.py ./glean-blame.tar searchfox.repositories glean-blame.tar
```

Equivalently, you can do it with the aws CLI tool, making sure to set the permissions:
```
cd $MOZSEARCH
aws s3 cp ./glean.tar s3://searchfox.repositories/glean.tar --acl public-read
aws s3 cp ./glean-blame.tar s3://searchfox.repositories/glean-blame.tar --acl public-read
```

## 4. Update the mozsearch-mozilla repo

For this you need to clone the [mozsearch-mozilla](https://github.com/mozsearch/mozsearch-mozilla) repo. For
convenience in step 5 below, it's best to do this inside the vagrant VM instance, like so:

```
cd /home/vagrant
git clone https://github.com/mozsearch/mozsearch-mozilla mozilla-config
```

and then modify the `config.json` file with an entry for new repo. A basic one might look like this:

```json
    "glean": {
      "index_path": "$WORKING/glean",
      "files_path": "$WORKING/glean/glean",
      "git_path": "$WORKING/glean/glean",
      "git_blame_path": "$WORKING/glean/glean-blame",
      "github_repo": "https://github.com/mozilla/glean",
      "objdir_path": "$WORKING/glean/objdir",
      "codesearch_path": "$WORKING/glean/livegrep.idx",
      "codesearch_port": 8088
    }
```

A couple things to note:
* The `codesearch_port` should be unique in the file, so increment by one compared to whatever the last entry in the file is.
* Watch your commas! This is JSON, so the last entry should not be followed by a comma.

You also need to create a folder for your repo, with the `setup`, `build`, `upload`, and `find-repo-files` scripts. You can
look at the existing folders for other repos for inspiration. Copy-pasting from something like the `glean` repo will probably
be a good start, although the `build` step may need to be modified depending on whether or not your repo is buildable and
produces useful artifacts on the searchfox indexing instance.

Try to ensure that the `setup` script avoids unnecessary re-work for subsequent invocations on a "dirty" tree.
In other words, the default operation of the `indexer-setup.sh` script will not clean the working directory, so
artifacts may left behind from a previous run of `indexer-setup.sh`, and the repo's `setup` script should try
to reuse those artifacts where possible, rather than failing.

Finally, update the top-level `help.html` file to include a link to your new repo as well.

## 5. Test and debug

Assuming you did step 4 inside the vagrant VM, you can use the `build-mozilla-repo` target in the Makefile to test out
and debug the indexing of your new repository. However this will do a lot of work, including all the mozilla-central
indexing, which you probably don't want as it takes a lot of time. So first edit the `/home/vagrant/mozilla-config/config.json`
file to just have the entry for your new repo (i.e. delete all the other entries). Also ensure the `default_tree` field
at the top of the file points to your repo. Then:

```
cd /vagrant
make build-mozilla-repo
```

This will spew out a lot of output as it does stuff, and either end in an error (which you will need to debug), or deploy
the web server in your vagrant VM which you will be able to access from http://localhost:16995/.

Once any issues are debugged, push a PR with your changes to the `mozsearch-mozilla` repo.

## 6. Update load balancer

If the repo is being added to a config file OTHER than config1.json, it will need an entry in the load balancer. This is
what tells AWS to route requests for this repo to the web-server that hosts it. Setting this up requires AWS access,
and is usually done via the web console:
- Log in to the AWS console at aws.sso.mozilla.org
- Go to the EC2 service, and then the "Load balancers" page from the sidebar.
- Select the "release-lb" balancer, and then edit the rules for the HTTP listener.
- Add a new rule (or edit an existing one) such that the path for your repo is forwarded to the appropriate release target group. Use the existing rules as guides. Note that each rule has a limit of 5 condition values (i.e. repos), which is why they sometimes spill over into new rules even though they have the same target.
- Repeat the previous step for the HTTPS listener on the "release-lb" balancer.
