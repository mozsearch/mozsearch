# Searchfox Testing

## Overview: Snapshot Testing

The primary testing mechanism both for development and detecting failures in
production is use of [insta](https://insta.rs/)-based snapshot testing.

Test cases are defined as pipelines to the `searchfox-tool` which are run
against the on-disk index and against the web-server.  The expected output of
the commands is checked into the repository, and tests confirm that the output
of test runs matches the expected output.  When changes are expected, a "review"
mode can be used to run the tests and see a diff of the changes in output that
the code author can confirm are expected and desired, and then the code reviewer
can confirm the diffs of the changes to the repository.

For the searchfox "tests" repository, the inputs and expected outputs can be
found at:
- https://github.com/mozsearch/mozsearch/tree/master/tests/tests/checks/inputs
- https://github.com/mozsearch/mozsearch/tree/master/tests/tests/checks/snapshots

For the mozilla-central production configuration, the inputs and expected
outputs can be found at:
- https://github.com/mozsearch/mozsearch-mozilla/tree/master/mozilla-central/checks/inputs
- https://github.com/mozsearch/mozsearch-mozilla/tree/master/mozilla-central/checks/snapshots

This mechanism was introduced in
[bug 1707282](https://bugzilla.mozilla.org/show_bug.cgi?id=1707282) and much of
the design rationale and longer term goals can be found in the bug there,
combined with comments and discussion in the
[initial PR](https://github.com/mozsearch/mozsearch/pull/422).

### Normalizations

Production repositories will inherently change over time.  In particular,
production blame and coverage data will continually change.  To this end, we
have implemented a `prod-filter` command that is used in our production checks
in order to:
- Strip .cov-strip elements.
- Strip .blame-strip elements.
- Replace line numbers with N
- Replace data-i values with "NORM".

## Purpose of the Checks

### mozsearch "tests" checks

These are regression tests that also may be used as a place one might be able to
quickly check what the current data representations look like.

Any changes to the indexing process should either be resulting in (expected)
changes to existing checks, or new checks should be introduced that cover the
new functionality.

Check coverage need not be exhaustive.  It's preferable that changes to the
expectations be something that a human can feasibly review in their entirety.
If you find yourself adding excessively verbose check expectations, it might
be appropriate to add functionality to searchfox-tool commands so that the
output can be filtered and/or automated invariant-checks can be performed
internally.

### production mozilla-central checks

The mozilla-central checks are intended to detect when changes in
mozilla-central break specific aspects of searchfox support.  For example,
changes in the auto-generated naming scheme for the C++ IPC bindings or C++
XIPDL or rust XPIDL support could break that specific class of support.  So we
pick specific real types and check for their expected definitions and related
state.

In the event the checks fail, the indexer fails and the previous web-server will
continue to run with its stale content.  An email will be sent to the searchfox
list, letting the maintainers know.  Usually regressions like this are simple
and straightforward, which means that it's not too bad to address the change.

Because we currently choose real types, there's also the potential for false
positives if the thing we chose gets renamed/refactored.  Again, this is
potentially quite quick to address.

#### Data Catch 22

Previously, our checks were just expecting the presence of a specific symbol in
a specific file, not matching exact output.  This made it easy to update the
check script without having any dependency on data that's only available on a
failed indexer.

Unfortunately, we now do have a data dependency, and our current indexer
behavior is to stop the indexer on failure, which makes it harder to get the
data off of it.

XXX the "release" condition for review versus fail didn't land that was proposed
at https://github.com/mozsearch/mozsearch/pull/425#issuecomment-898745607 and
that should help address this problem.  It's mainly a question of making sure
that we propagate `INSTA_FORCE_PASS=1` to the invocation of check-index.sh
for non-release builds.  As discussed below, it could be good to also propagate
the additional resulting files that `cargo insta review` could then process.

#### Needed Enhancements

We likely have additional work to do like to have the failed indexer generate a
tarball of its deviation from expectations and to upload this to S3.  Or to
fulfill the original plan of having non-release channel builds not fail

## Updating Checks

### mozsearch "tests" repo

Inside the VM, cd to `/vagrant` and then run:
```
make review-test-repo
```

This will trigger the [cargo insta](https://insta.rs/docs/cli/) review
mechanism which will make the appropriate changes to the repository for you to
commit.

### production mozilla-central checks

#### Setup

Note: You don't have to do things the following way; the underlying mechanism is
reasonably straightforward, but this is the current approach used by asuth and
what he will copy and paste from.

Check out mozsearch-mozilla as `config` inside your mozsearch checkout:
```
git clone https://github.com/mozsearch/mozsearch-mozilla.git config
```

This will also expose it at `/vagrant/config` inside the VM because of the (NFS)
mount in use.  Note that the VM will also have made its own checkout at
`~/config` but that directory isn't exposed outside the VM and so isn't useful.

You probably will then want to add your own fork as a remote.  For example,
assuming you are asuth, you would do:
```
git remote add gh-asuth git@github.com:asutherland/mozsearch-mozilla.git
```

#### Updating

Outside the VM, make sure you have an up-to-date copy of the default branch and
then branch from that.
```shell
# change into the mozsearch-mozilla checkout
cd config
# get off of any existing branch
git checkout master
# update the default branch
git pull origin master
# make our new branch
git checkout -b update-checks
```

Inside the VM:
```shell
# change into the mozsearch-mozilla checkout dir in the VM
cd /vagrant/config
# run the checks from this repo against the current state of
# https://
./review-build-check-results.sh config1.json mozilla-central release
```

Then, outside the VM, commit the changes to the branch and create a pull
request and submit it.

#### Updating checks on a stopped AWS indexer due to local (disk) check failure

Currently, in the event the indexer has a check failure, it will stop before
starting the web-server, which means you can't use the
`review-build-check-results.sh` script from your local machine to talk to a web
server.

In this case if you login, you can run the following to be able to reproduce the
failures experienced by the indexer run:

```shell
# mount the index to ~/index as documented in aws.md
sudo mount /dev/`lsblk | grep 300G | cut -d" " -f1` /index
# make index-scratch paths valid again
sudo ln -s /index/interrupted /mnt/index-scratch

export MOZSEARCH_PATH=~/mozsearch
export CONFIG_REPO=~/config
$MOZSEARCH_PATH/scripts/check-index.sh /index/interrupted/config.json mozilla-central "filesystem" ""
```

If we want to update the checks in the config, we can re-run with
`INSTA_FORCE_PASS=1` like so:

```shell
INSTA_FORCE_PASS=1 $MOZSEARCH_PATH/scripts/check-index.sh /index/interrupted/config.json mozilla-central "filesystem" ""
```

If we want to review these changes on the machine, we can do:
```shell
cargo insta review --workspace-root=$CONFIG_REPO
```

Now we've updated the mozsearch-mozilla repo checked out at ~/config on the
server, but we still need to get that committed to github somehow, and you
almost certainly don't want the indexer server to have access to your normal
github creds (although I guess we could give the indexer its own login?), so
the easiest thing to do is run the following locally to copy the changed
contents to your local machine (after doing `cargo insta review` above):

```shell
export INSTANCE=<you gotta get this from ssh.py or channel-tool.py>
# note this assumes mozilla-central; change as appropriate
infrastructure/aws/scp-while-sshed.py $INSTANCE 'config/mozilla-central/checks/snapshots/*' config/mozilla-central/checks/snapshots
# if you changed any of the inputs, you'll want to run this too:
infrastructure/aws/scp-while-sshed.py $INSTANCE 'config/mozilla-central/checks/inputs/*' config/mozilla-central/checks/inputs
```
