# Adding a new repository

The basic steps needed to add a new repository to searchfox are listed below. This assumes as a prerequisite
that you have set up the Vagrant VM as documented in the [top level README](../README.md).

## 1. Create a tarball with the git repo

This is as simple as running `git clone` to clone the repo locally, and then running `tar` to make a tarball.
Note that it's best to do this inside your mozsearch folder so that it's automatically mirrored inside the
vagrant VM instance for step 2 below. For example:

```
cd $MOZSEARCH
git clone https://github.com/mozilla/glean
tar cf glean.tar glean
```

## 2. Create a tarball of the blame repo

To create the blame repo, you can manually run the `blame/transform-repo.py` script. Note that this requires pygit2 to be
available to Python, so it's easiest to run it inside the vagrant instance. Note that if there is an hg canonical repo
for the repo you want to index, you can provide a git <-> hg mapping file as a third argument to the `transform-repo.py`
script to include hg revision identifiers in the index.

```
cd /vagrant
mkdir glean-blame
pushd glean-blame && git init . && popd
python blame/transform-repo.py glean glean-blame # this might take a while, depending on your repo size
tar cf glean-blame.tar glean-blame
```

## 3. Upload the two tarballs to the S3 bucket

This uses the `upload.py` script, and assumes you have appropriate AWS permissions (see the section on
[Setting up AWS access locally](aws.md#setting-up-aws-locally)). These commands should be run in whatever
environment you have AWS access with (typically outside the vagrant VM). If you do not have AWS access,
please contact one of the searchfox maintainers who can do the upload for you so you can finish debugging
and testing the setup.

```
cd $MOZSEARCH
python infrastructure/aws/upload.py ./glean.tar searchfox.repositories glean.tar
python infrastructure/aws/upload.py ./glean-blame.tar searchfox.repositories glean-blame.tar
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
the web server in your vagrant VM which you will be able to access from http://localhost:8001/.

Once any issues are debugged, push a PR with your changes to the `mozsearch-mozilla` repo.
