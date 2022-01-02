# Cross-referencing analysis data

Once analysis results for individual files have been generated, these
results need to be combined to make it possible to link from one file
to another. This is the job of the cross-referencer (located in
`tools/src/bin/crossref.rs`). It reads in target records from every
analysis file and records them in a hashtable. The hashtable maps the
symbol name to every target record with that symbol. Finally, the
hashtable is written to a pair of files `${index}/${tree_name}/crossref` and
`${index}/${tree_name}/crossref-extra`.  The `-extra` variant stores data
payloads that are large enough that they impact our ability to perform a
memory-mapped binary search of `crossref` file.

The general structure of hit records is:

```
{<kind>: [{"path": <file-path>, "lines": [{"lno": <lineno>, "line": <text-of-line>}, ...]}, ...]}
```

The values for `<kind>` are Declarations, Definitions, Uses,
Assignments, IDL, and Callees.

The `<text-of-line>` contains the text of the given line, with leading and
trailing spaces stripped.  An example entry in this file looks like:

```
!_ZN19nsISupportsPRUint647SetDataEm
:{"Declarations":[{"lines":[{"line":"NS_IMETHOD SetData(uint64_t aData) = 0;","lno":830},{"line":"NS_IMETHOD SetData(uint64_t aData) override; \\","lno":842}],"path":"__GENERATED__/dist/include/nsISupportsPrimitives.h"}],"Definitions":[{"lines":[{"line":"nsSupportsPRUint64::SetData(uint64_t aData)","lno":371}],"path":"xpcom/ds/nsSupportsPrimitives.cpp"}],"IDL":[{"lines":[{"line":"attribute uint64_t data;","lno":129}],"path":"xpcom/ds/nsISupportsPrimitives.idl"}],"Uses":[{"lines":[{"line":"wrapper->SetData(mWindowID);","lno":72}],"path":"dom/audiochannel/AudioChannelService.cpp"},{"lines":[{"line":"wrapper->SetData(mID);","lno":8925}],"path":"dom/base/nsGlobalWindow.cpp"},{"lines":[{"line":"ret->SetData(gBrowserTabsRemoteStatus);","lno":1004}],"path":"toolkit/xre/nsAppRunner.cpp"}]}
```

The file is sorted.

The first line is the symbol name and the second line is a JSON object
describing all the target records for that symbol.  Each line begins with an
indicator character that identifies the type of line.

More details from https://bugzilla.mozilla.org/show_bug.cgi?id=1702916:
- `crossref` continues to be newline-delimited.
- Each line in `crossref` gets a prefix indicating what's on the line:
  - `!`: An Identifier follows.
  - `:`: Inline-stored JSON for the preceding line's identifier (which must be an identifier).
  - `@`: Externally-stored JSON in `crossref-extra`.  The entirety of the line (eliding the trailing newline) should be `@${offsetOfJsonOpeningCurlyBrace.toString(16)} ${lengthIncludingNewline.toString(16)}`.  The offset and length (including newline) are represented in hexadecimal (without preceding `0x`) and separated by a space.  The choice of hex is for information density purposes while still being human readable.  Because I'll be augmenting `searchfox-tool` to directly perform any lookups people would otherwise use UNIX tools for, I think this should be fine.
- Although it seems like this would support having comment lines, we won't support
  these, at least not initially, as it would complicate the bisection logic which
  benefits from being able to depend on things being written in pairs.
- `crossref-extra` also ends up looking like `crossref` for the sake of ease of debugging.  It's newline delimited and will include (useless) `!Identifier` lines preceding each long JSON line.  The JSON lines also get `:` prefixed onto them even though the offsets in `crossref` will not include the leading `:`.
  - The rationale here is that it seems nice if someone wants to build a naive script / grep command invocation that they can just point it at both files and they'll get a result without having to deal with the offset indirection by requiring the second line to start with `:` and ignore the `@` second lines.
- The initial arbitrary line length cutoff will be 3k based on the statistics I gathered from comment 0 and because if we assume 4k page sizes that means in any 4k page we should then still be able to find an identifier (although the binary search will likely be naive about page alignment issues which means it would probably be happier with a constant that's less than 2k).  I'm sure one could write a nice shell script to brute force some practical legwork.  Or we could vary the constant randomly every day and gather the performance characteristics, etc. etc.  I'm not super concerned, I just want rust-based lookups.

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
