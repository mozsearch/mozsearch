searchfox-tool is both a rust binary that we build that can be directly run
against live servers and local index data, as well as the underlying mechanism
of the [testing mechanism](testing-checks.md).

This document is intended to be a repository of searchfox-tool command lines
that we've used as a copy-and-paste reference and starting point for similar
explorations.

## searchfox-tool parsing oddities

searchfox-tool is built around the idea of building pipelines, and its primary
use-case is the test check mechanism that doesn't directly involve a shell, so
it very unusually requires you to provide it with a single quoted argument which
it will then apply shell rules to.

As an example, all of the following will work:

A single argument, this is fine!
```
searchfox-tool --help
```

Even though the intent is for this to be 2 arguments, we have to quote them into
a single argument.
```
searchfox-tool 'query --help'
# `searchfox-tool query --help` however, would not!  you get an error!
```

## searchfox-tool is self-documenting

Run the following to get a list of subcommands you can chain together:
```
searchfox-tool '--help'
```

Once you know a subcommand, you can get help on it.  For example, choosing
"query":
```
searchfox-tool 'query --help'
```

## Cookbook Proper

### Dumping crossref info from an identifier on a web-server shell
```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central search-identifiers ServiceWorkerManager::SendNotificationClickEvent | crossref-lookup'
```

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central search-identifiers PClientSourceParent::SendPClientSourceOpConstructor | crossref-lookup' | jq .
```

### Dumping crossref info from a symbol on a web-server shell


```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central crossref-lookup "_ZN7mozilla3dom18ClientSourceParent7StartOpEONS0_23ClientOpConstructorArgsE"' | jq .
```

### Graph traversal without rendering on a web-server shell

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central search-identifiers ServiceWorkerManager::SendNotificationClickEvent | crossref-lookup | traverse --edge=uses --max-depth=2' | jq .
```

```
RUST_LOG=trace ~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central search-identifiers ClientSource::Focus | crossref-lookup | traverse --edge=uses --max-depth=4'
```

### Graphing on a web-server shell

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central search-identifiers mozilla::GetContentWin32kLockdownEnabled | crossref-lookup | traverse --edge=uses --max-depth=9 | graph --format=svg'
```

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=mozilla-central search-identifiers GetLiveWin32kLockdownState | crossref-lookup | traverse --edge=uses --max-depth=9 | graph --format=svg'
```


### Debugging the field layout table locally

```
/vagrant/tools/target/release/searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests search-identifiers field_layout::template_base::Base | crossref-lookup | format-symbols'
```


### Diffing Query Results

While investigating aspects of queries that hit limits because of non-intuitive
results, a `--diff` flag was added to query that allows it to be used in a
pipeline like `searchfox-tool 'query foo | query --diff foot'` to diff the
change in results.  This was not immediately useful for 2 reasons:
1. The "bounds" fields generated a ton of noise because the substring lengths
   were different.  So `--normalize` was added to compensate for that.
2. Our search result hit lists are normally returned as arrays where the JSON
   diff algorithm correctly treats the ordering as important.  So `--dictify`
   was added as a means of transforming `[{ "path": "foo", ... }]` to
   `{ "foo": { "path": "foo", ... }}` which the JSON diff algorithm then applies
   set/map semantics to, which is what we as humans like.

So the specific example was for "new xmlhttprequest" with and without the
trailing "t".  In this case, we were seeing different result counts, which was
surprising because you would expect a situation that is not showing evidence of
hitting a result limit based on the understood logging output.  (It of course
turns out that we were hitting a result limit count and the misleading counts
were due to aggregating based on the path first.)

```
searchfox-tool 'query "new xmlhttpreques" | query --diff --normalize --dictify "new xmlhttprequest"'
```

### Test Server Text Search
```
RUST_LOG=trace ./searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests search-text searchfox'
```


### Graphing Test Server contents

From inside the VM:
```
./searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests search-identifiers outerNS::OuterCat::meet | crossref-lookup | traverse | graph --format=svg' > /vagrant/pretty.svg
```
