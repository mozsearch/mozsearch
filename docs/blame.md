# Blame

In order to serve up blame information quickly, mozsearch caches blame
data for every version of every file in a repository. Here's how it works:

Suppose a file has three revisions:

```
rev1:
abc
def
ghi

rev2:
abc
def2
ghi

rev3:
abc
ghij
```

Then mozsearch will generate three "blame revisions" in a new repository:

```
blame-rev1:
rev1
rev1
rev1

blame-rev2:
rev1
rev2
rev1

blame-rev3:
rev1
rev3
```

Every revision in the original repository has a corresponding blame
revision. The blame version of the file will have one line for every
line in the original file. This line will contain the revision ID of
the revision in the original repository that introduced that line.

This data is stored in its own git repository. This repository, called
the blame repository, has exactly the same file structure as the
original repository. Each commit in this repository corresponds to a
commit in the original repository. Commit messages in the blame
repository give the revision they correspond to in the original
repository.

Let's imagine you want to get blame for a file `${f}` at revision
`${rev}`. First, find the revision `${blame_rev}` in the blame
repository that corresponds to `${rev}` (the web server keeps a
mapping between revisions in memory for this purpose). Then find
`${f}` in the blame repository at revision `${blame_rev}`. Finally,
show these two files side-by-side and you're done.

Mozsearch uses the `blame/transform-repo.py` tool to generate a blame
repository. Generating cached blame information is pretty
slow. However, each indexing run only needs to generate new blame
revisions for each new revision that has appeared in the original
repository since the last indexing. Typically the blame repository is
about the same size as the original repository since it compresses
very well with git's delta compression.
