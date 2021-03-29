# Cross-referencing analysis data

Once analysis results for individual files have been generated, these
results need to be combined to make it possible to link from one file
to another. This is the job of the cross-referencer (located in
`tools/src/bin/crossref.rs`). It reads in target records from every
analysis file and records them in a hashtable. The hashtable maps the
symbol name to every target record with that symbol. Finally, the
hashtable is written to a file `${index}/${tree_name}/crossref`. The
general structure is:

```
{<kind>: [{"path": <file-path>, "lines": [{"lno": <lineno>, "line": <text-of-line>}, ...]}, ...]}
```

The values for `<kind>` are Declarations, Definitions, Uses,
Assignments, IDL, and Consumes.

The `<text-of-line>` contains the text of the given line, with leading and
trailing spaces stripped.  An example entry in this file looks like:

```
_ZN19nsISupportsPRUint647SetDataEm
{"Declarations":[{"lines":[{"line":"NS_IMETHOD SetData(uint64_t aData) = 0;","lno":830},{"line":"NS_IMETHOD SetData(uint64_t aData) override; \\","lno":842}],"path":"__GENERATED__/dist/include/nsISupportsPrimitives.h"}],"Definitions":[{"lines":[{"line":"nsSupportsPRUint64::SetData(uint64_t aData)","lno":371}],"path":"xpcom/ds/nsSupportsPrimitives.cpp"}],"IDL":[{"lines":[{"line":"attribute uint64_t data;","lno":129}],"path":"xpcom/ds/nsISupportsPrimitives.idl"}],"Uses":[{"lines":[{"line":"wrapper->SetData(mWindowID);","lno":72}],"path":"dom/audiochannel/AudioChannelService.cpp"},{"lines":[{"line":"wrapper->SetData(mID);","lno":8925}],"path":"dom/base/nsGlobalWindow.cpp"},{"lines":[{"line":"ret->SetData(gBrowserTabsRemoteStatus);","lno":1004}],"path":"toolkit/xre/nsAppRunner.cpp"}]}
```

The first line is the symbol name and the second line is a JSON object
describing all the target records for that symbol.  The file is sorted.

### Identifiers file

In addition, an identifiers file is generated that is used for
`id:`-style searches. It appears at
`${index}/${tree_name}/identifiers`. For each target record, the
`pretty` name of the identifier is broken into components by splitting
on `:` and `.`. Given a `pretty` name of `A::B::C`, lines are
generated for `A::B::C`, `B::C`, and `C` (since these are the things
people might search on). The line has the form:

```
<qualified-name-suffix> <symbol-name>
```

This file is sorted (case insensitively). When the user searches for a
qualified name `Abc::Def`, the web server will use binary search to
find all lines starting with `Abc::Def`. Then it looks up the
corresponding symbols in the crossref file and combines those results.

### Jumps file

Finally, a `jumps` file is also generated. This file is used when
generating the "Goto XYZ" context menu items. We only generate one of
these items if there is exactly one definition of a given symbol. For
all such symbols, we generate a line in the `jumps` file of the
following form:

```["<symbol-name>","<definition-path>",<definition-lineno>,"<definition-pretty-name>"]```

The pretty name comes from the `pretty` property of the single target
record for the definition.
