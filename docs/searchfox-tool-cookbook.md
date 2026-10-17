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
--tree=firefox-main search-identifiers ServiceWorkerManager::SendNotificationClickEvent | crossref-lookup'
```

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=firefox-main search-identifiers PClientSourceParent::SendPClientSourceOpConstructor | crossref-lookup' | jq .
```

### Dumping crossref info from a symbol on a web-server shell


```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=firefox-main crossref-lookup "_ZN7mozilla3dom18ClientSourceParent7StartOpEONS0_23ClientOpConstructorArgsE"' | jq .
```

### Graph traversal without rendering on a web-server shell

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=firefox-main search-identifiers ServiceWorkerManager::SendNotificationClickEvent | crossref-lookup | traverse --edge=uses --max-depth=2' | jq .
```

```
RUST_LOG=trace ~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=firefox-main search-identifiers ClientSource::Focus | crossref-lookup | traverse --edge=uses --max-depth=4'
```

### Graphing on a web-server shell

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=firefox-main search-identifiers mozilla::GetContentWin32kLockdownEnabled | crossref-lookup | traverse --edge=uses --max-depth=9 | graph --format=svg'
```

```
~/mozsearch/tools/target/release/searchfox-tool '--server=/home/ubuntu/index/config.json
--tree=firefox-main search-identifiers GetLiveWin32kLockdownState | crossref-lookup | traverse --edge=uses --max-depth=9 | graph --format=svg'
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

### Block diagrams

crossref rolls the calls between functions up into calls between classes and
between subsystems (from files' Bugzilla components), stored as `callRollup`
on classes' crossref entries and on subsystems' pseudo-symbols
(`SUBSYS_<subsystem>`, and `SUBSYS_` for the whole tree; see
`tools/src/file_format/rollup.rs`).  `block-diagram` draws them, without
walking methods:
```
./searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests block-diagram | graph --format=svg --hier=flat'
./searchfox-tool "--server=/home/vagrant/index/config.json --tree=tests block-diagram 'Core/Rollup View' | graph --format=svg --hier=subsystem"
./searchfox-tool "--server=/home/vagrant/index/config.json --tree=tests block-diagram T_rollup::Panel | graph --format=svg"
./searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests crossref-lookup SUBSYS_'
```
The whole tree's subsystems; a subsystem's classes and their neighbors
(`--level subsystem` for its neighboring subsystems instead); a class's
neighbors.  Edges are labeled and weighted by how many calls they stand for;
`--min-count` and `--max-edges` prune them.  The query terms are
`block-diagram:*`, `block-diagram:<subsystem or class symbol>`,
`block-level:`, `block-min-count:` and `block-max-edges:`.

### Following values (dataflow)

`dataflow` (see `tools/src/dataflow/`) shows what it understood of a file's
functions, or follows a value forwards (where does it go?) or backwards (where
does it come from?), as text, DOT, SVG, or JSON:
```
./searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests dataflow dataflow/dataflow.cpp --function GetBoolWithDefault'
./searchfox-tool '--server=/home/vagrant/index/config.json --tree=tests dataflow dataflow/dataflow.cpp --function UseDefault --backward value'
./searchfox-tool "--server=/home/vagrant/index/config.json --tree=tests dataflow --at dataflow/dataflow.cpp#46 --symbol 'F_<T_Widget>_mName' --direction backward --format dot"
```
Places are named as the IR shows them (`aRetVal`, `*aRetVal`, `this->mName`,
`return` for backwards from the returns); `--symbol` with `--at <path>#<line>`
takes a local's, parameter's, field's or global's symbol instead, as the
context menu's "Follow" items do through the `follow-forward:`,
`follow-backward:` and `follow-at:` query terms.  `--changed
"<path>:<lines>;…"` slices changed lines into clusters, `--questions <paths>`
lists questions for an oracle, `--oracle <dir>` applies its answers, and
`--stats <paths>` reports how much the frontends understood.

`--summary` shows a file's (or `--function`'s) dataflow summaries (see
`tools/src/dataflow/summary.rs`): which inputs (parameters, fields, globals,
calls' results) flow into which outputs (fields, returns, calls' arguments),
and whether the function is a getter, setter, trivial, small or big; and
`--field-edges <paths>` composes the summaries of those files' functions
through their calls into edges between fields (and between functions and
other objects' fields).  Trees with `"dataflow_summaries": true` precompute
both for every file (`dataflow-summarize`, before crossref), and crossref
rolls the edges up into classes' and subsystems' `dataRollup`s, which
`block-diagram --edges data` (`block-edges:data`) draws instead of calls;
`--no-accessors` (`block-no-accessors:`) doesn't count calls of getters,
setters and trivial methods.
