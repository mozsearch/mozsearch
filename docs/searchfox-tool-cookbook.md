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

### Working over a remote tree

With a URL for `--server`, the tool works over that server's tree through its
data API (see `docs/web-server.md`), without an index of your own, ex: a
dataflow trace over firefox-main:
```
searchfox-tool "--server=https://searchfox.org/ --tree=firefox-main --remote-cache=$HOME/.cache/searchfox-remote dataflow dom/workers/WorkerPrivate.cpp --function WorkerPrivate::Notify --forward aStatus"
```

Responses are only cached on disk where `--remote-cache` (or the
`SEARCHFOX_REMOTE_CACHE` environment variable) says, under
`{host}/{tree}/{indexed revision}/`, and the cache keeps only the tree's
current indexed revision.  Without either, nothing is cached.

The pipeline-server can serve `/query/` over remote trees too, without any
configuration of its own, given their URLs (separated by semicolons) instead
of a config file; their settings come from their servers' `config` routes:
```
PIPELINE_SERVER_ADDR=127.0.0.1:8002 SEARCHFOX_REMOTE_CACHE=$HOME/.cache/searchfox-remote tools/target/release/pipeline-server "https://searchfox.org/firefox-disco/;https://searchfox.org/wubspecs/"
```
and `PIPELINE_SERVER=http://localhost:8002 node scripts/dev-server.js` sends
the pages' queries (diagrams, dataflow, slicing) to it.  The web-server takes
the trees' URLs too (`WEB_SERVER_ADDR=127.0.0.1:8001 web-server <urls>
<status-file>`), and builds its pages over the trees' servers (revisions'
files and directories, history, diffs, commits, interdiffs, outline diffs,
try pushes and their canvases, explore pages, blame popups' data,
completions; `WEB_SERVER=http://localhost:8001` sends dev-server.js's
requests for them to it); its rendered and algorithm views and diagnostics
still need the trees' files.

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
between subsystems (from files' Bugzilla components; in trees whose files
have none, directories: a file's top-level directory, split into
subdirectories where it has more than `max(256, files / 32)` files), stored as `callRollup`
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
A call of an IPDL message's generated `Send` method counts as a call of the
actors' `Recv` implementations (an `ipc` count on the edge, drawn dotted
when it's all IPC); notifying observers of a topic as a call of the
observers' methods receiving it; and a JS call of an IDL method's binding
(or of an XPIDL member's generated TypeScript type) as a call of its C++
slot (an XPIDL interface's method, as C++ callers through the interface
call it, or a WebIDL member's implementation).  The whole tree's subsystems; a subsystem's classes and their neighbors
(`--level subsystem` for its neighboring subsystems instead); a class's
neighbors.  Edges are labeled and weighted by how many calls they stand for;
`--min-count` and `--max-edges` prune them.  The query terms are
`block-diagram:*`, `block-diagram:<subsystem or class symbol>`,
`block-level:`, `block-min-count:` and `block-max-edges:`.  `--with <class
or subsystem>` (`block-with:`) keeps only the edges between the focus's side
and it: for an edge between two classes, `block-diagram <A> --level field
--with <B>` is the data between their fields and methods, and for one
between two subsystems, `block-diagram <S1> --with <S2>` the calls (or, with
`--edges data`, data) between their classes; the context menus of block
diagrams' edges offer them.

A try push's semantic delta (see `tools/src/try_delta.rs`) carries what it
changes about the call roll-ups: the roll-ups of the changed files' calls
before and after the push (`callRollupDelta`), which a query in the push's
context (`rev=<head>`) subtracts from and adds to the tree's.  Data roll-ups
stay the tree's (a push's summaries would need the whole tree's).

`traverse --paths-between --rollup-guide` (the `rollup-guide:true` query term,
with `calls-between:`) follows only callers that are methods of classes on a
path between the sources' and targets' classes in the call roll-ups (and
their supers and subclasses), or that aren't methods, so it looks up fewer
symbols.

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
"<path>:<lines>;…"` slices changed lines into clusters, `--slice-rev <rev>`
a revision's changes (a commit's, or `<a>..<b>`'s, in the coordinates of the
revision the server analyzes: the indexed one, or, for queries with `rev`, a
try push's head; see `tools/src/rev_lines.rs`), as the `slice-rev:` query
term does (`slice-cluster:<n>` for one cluster, `slice-format:json` for the
units, their functions, classes and commits, and links; the pipeline-server
caches slicings, see `tools/src/step_cache.rs`); in a try push's context, the
lines it removes are units too, from the indexed revision's analysis,
`--questions <paths>`
lists questions for an oracle, `--oracle <dir>` applies its answers, and
`--stats <paths>` reports how much the frontends understood.

Values go through IPDL structs' fields (by ipdl-analyze's binding slots)
rather than into the C++ IPDL generates for them: a field's value comes from
its struct's constructors' arguments (at the field's index) and assignments
through its mutable accessor (`x.f() = v`), and goes to its accessors' calls.
A local's or parameter's struct's field is followed along the struct
(`x.<field>`, through calls and IPC, as places' parts are), so a value put
in a struct reaches the reads of that struct's field after it's sent, and a
read comes from the writes of that struct's field before it was; a field of
a struct in `this`'s or a global's field, a read of a struct that isn't a
place, and a `.ipdlh` field followed from its declaration go to (or come
from) the field's accessors' calls wherever they are.  `--symbol` takes an
accessor's symbol, or a `.ipdlh` field's `IPDL_` symbol, too, and slicing
links units which read or write the same IPDL field, as it does for fields.

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
setters and trivial methods.  Fields (and functions) have data roll-ups of
their own edges, which `block-diagram <class> --level field`
(`block-level:field`, with `hier:pretty` for classes' tables of fields)
draws: how data moves between the class's fields and other classes' fields
and functions.  Summaries see through the code IPDL generates, as traces
do: an IPDL struct's field (or union's variant) and a message's parameter
are data of their own (written by the struct's constructors, `x.f() = v` and
`Send` calls' arguments, read by its accessors and the `Recv`
implementations' parameters), so data edges cross IPC through them, and the
IPDL struct or message is a class between the sender's and the receiver's
in roll-ups (`block-diagram IPDL_<struct> --level field` draws its fields,
and `block-diagram IPC_<message> --level field` its parameters).
Accessors' summaries (getters', setters' and trivial methods', with the
constants they return or write) are in their crossref entries (`dataflow`),
and traces step over their calls by them rather than building their files'
IR (often a big header's, for inline getters), showing each as one node at
its definition, and skip their uses of fields they don't write when looking
for a field's writes; `--no-summaries` enters them, and slicing always does
(its units link by traces' nodes on their lines).
