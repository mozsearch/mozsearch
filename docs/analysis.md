# Analysis JSON

All data from the semantic analysis of a source file is dumped out to
JSON. For a file with path `${path}` from the repository root, the
analysis data is stored at `${index}/${tree_name}/analysis/${path}`.

The analysis data is broken into records. Each record corresponds to
an identifier in the original source code. Each line of the file
contains a record. (Technically the file is not JSON itself, but a
series of JSON objects delimited by line breaks.)

Analysis records are currently generated from:
* `scripts/js-analyze.js` for the JavaScript analysis.
* `scripts/idl-analyze.py` for IDL files.
* `clang-plugin/MozsearchIndexer.cpp` for C++ files.
* `tools/src/bin/rust-indexer.rs` for Rust files.

Analysis records may also be downloaded from Taskcluster for
mozilla-central builds (this can be viewed as an optimization to
avoid rebuilding mozilla-central as part of indexing). In fact,
records may be downloaded for multiple platforms, in which case
the `scripts/merge-analyses.py` script is used to combine the
different analyses from different platforms for a given source
file.

Analysis records are consumed from Rust code in
`tools/src/analysis.rs`.

There are two kinds of records: sources and targets. Each source
record generates one or more context menu items when the user clicks
on an identifier. Each target record corresponds to a place in the
source code where things are to be found. For the most part, there
will be one source record and one target record for a given
identifier. However, in the case of C++ inheritance, there may be one
source record and multiple target records.

Here is an example analysis. First, some JavaScript code:

```
let x = {a: 1};
dump(x.a);
```

Then its analysis:

```
{"loc":"1:4-5","source":1,"syntax":"def,prop","pretty":"property x","sym":"#x"}
{"loc":"1:4","target":1,"kind":"def","pretty":"x","sym":"#x"}
{"loc":"1:9-10","source":1,"syntax":"def,prop","pretty":"property a","sym":"#a"}
{"loc":"1:9","target":1,"kind":"def","pretty":"a","sym":"#a"}
{"loc":"1:9-10","source":1,"syntax":"def,prop","pretty":"property x.a","sym":"x#a"}
{"loc":"1:9","target":1,"kind":"def","pretty":"x.a","sym":"x#a"}
{"loc":"2:0-4","source":1,"syntax":"use,prop","pretty":"property dump","sym":"#dump"}
{"loc":"2:0","target":1,"kind":"use","pretty":"dump","sym":"#dump"}
{"loc":"2:5-6","source":1,"syntax":"use,prop","pretty":"property x","sym":"#x"}
{"loc":"2:5","target":1,"kind":"use","pretty":"x","sym":"#x"}
{"loc":"2:7-8","source":1,"syntax":"use,prop","pretty":"property a","sym":"#a"}
{"loc":"2:7","target":1,"kind":"use","pretty":"a","sym":"#a"}
{"loc":"2:7-8","source":1,"syntax":"use,prop","pretty":"property x.a","sym":"x#a"}
{"loc":"2:7","target":1,"kind":"use","pretty":"x.a","sym":"x#a"}
```

### Locations

In JavaScript, there is one source and one target for each
identifier. Both kinds of records include a location, of the form
`${lineno}:${colno}` for targets and
`${lineno}:${start_colno}-${end_colno}` for sources.

### Symbols

Both kinds of nodes also contain a `sym` property, which is how
sources are linked to targets. Source nodes are allowed to contain a
comma-delimited list of symbols. The results for all of these symbols
are combined in search results. Target records can only contain a
single symbol.

In JavaScript, the symbol can take three forms:

* `${file_index}-${var_index}`. All local variables are assigned a number
within the file. In addition, each JS file has a unique number assigned to it.
Combining them, we can generate a unique symbol for every variable in the
repository. Note that top-level variables are considered properties of the
global object, so they use the next form.

* `#prop`. A property `prop` of an object is given the symbol `#prop`. Properties
used in different files will get the same symbol, since they might be for the
same object.

* `object#prop`. In some cases, mozsearch is able to infer a static name for the
object in which a property lives. It can do this for object literals
(i.e., `let x = {prop: ...};`) as well as cases like `Foo.method = ...;`.
These names aren't always useful, but they often are. In all such cases, mozsearch
also generates analysis records for the bare property names (`#prop`).

There are a variety of symbol names for C++ code:

* For functions, the mangled name of the function is used. This allows mozsearch to
distinguish overloads.

* For local variables, the symbol is `V_${variable_location_hash}_${variable_name_hash}`,
where the location is the hash of the filename and line where the variable is declared.

* For anonymous types, the symbol is `T_${type_location_hash}`.

* For named types, the symbol is `T_${qualified_type_name}`.

* For anonymous namespaces, the symbol is `NS_${namespace_location_hash}`.

* For named namespaces, the symbol is `NS_${qualified_namespace_name}`.

* For fields of structs, unions, and classes, the symbol is
`F_<${record_symbol}>_${field_index}`, where `${record_symbol}` uses
the recursively defined symbol name and `${field_index}` is the
index of the field starting from 0.

* For enumeration constants, the symbol is
`F_<${enum_symbol}>_${constant_name}`.

Note that in some cases, the symbol for a C++ identifier might vary from one
platform to another. For example, a function signature that includes `uint64_t`
produces a different mangled name on macOS and Linux. In such cases, if
analysis records are generated for multiple platforms, the final source records will
contain the symbols from all the platforms merged into a comma-separated list. The
`scripts/merge-analyses.py` script is used to do this.

Also, in many cases (notably for C++ and JS code), local variables have their
target records omitted, and their source records have the `no_crossref`
property. Since local variables are not referenced outside of a very narrow
context, this optimization helps to avoid bloating the index files with a
lot of unnecessary entries.

### Sources

A source record additionally contains a `syntax` property, a `pretty` property,
an optional `no_crossref` property, and an optional `nestingRange` property.

The `syntax` property describes how the identifier should be syntax highlighted.
It is a comma-delimited list of strings. Currently the only strings that have any
effect are:

* `def`, `decl`, and `idl` all cause the identifier to be shown in bold.
* `type` causes the identifier to be shown in a different color.

The `pretty` property is used to generate the context menu items for
the identifier. It should contain a human-readable description like
`constructor nsDocShell::nsDocShell` or `property
SessionStore.getTabState`.

The `no_crossref` property, if set, always has a value of `1`, and indicates
that this identifier will have no target records and does not participate
in cross-referencing.

The `nestingRange` property, if present, contains a value analogous to a
Clang SourceRange, with a string representation of "line1:col1-line2:col2" where
lines are 1-based and columns are 0-based.  This currently powers
"position:sticky" source code display so that when you are inside hierarchically
nested definitions you can immediately understand where you are and what the
scope is without needing to manually scroll up.

Ideally, the nesting range's two points are the start of the token creating a
nested block and the start of the token ending a nesting block.  ("{" and "}"
in C++ and the 'b' in "begin" for Pascal.  This is consistent with how Clang's
AST representations.)  However, when an analyzer doesn't have an exact AST to
work with (ex: rust as of writing this), we may do our best to simply specify a
conservative range of lines covering the children of a definition.

In cases where we have accurate nestingRange information, we may be able to do
neat tricks like highlight the area between braces or implement code folding.

#### Experimental / in flux
These fields, like the "structured" record type, are in flux as part of work
on the fancy branch.

A `type` may be emitted which is a string representation of the compiler's
understanding of the type/return type of something.  This will include
qualifiers like const/it's a pointer/it's a reference.

A `typesym` symbol may be emitted when the type corresponds to a type indexed by
searchfox (which will inherently not include qualifiers unless we're talking
about method signatures).

### Targets

Target records additionally contain a `kind` property, a `pretty` property,
and optionally `context`, `contextsym`, and `peekRange` properties.

The `kind` property should be one of `use`, `def`, `decl`, `assign`,
or `idl`. This property determines whether the identifier will appear
under the "Uses", "Definitions", "Declarations", "Assignments", or
"IDL" category of the search results page.

The `pretty` property is also used for the context menu. If a target
record is the only `def` target for a given symbol, then the context
menu for any source records with that symbol will contain a `Go to
${pretty}` entry, where `${pretty}` is the target's `pretty` property.

The `context` and `contextsym` properties capture an enclosing context
for the identifier, such as the enclosing function. These are used to
link to the context in search results that include the target record.

The `peekRange` property is a range of lines that appears to be
currently unused.

### Structured Records

Structured records are an attempt to provide richer information about types and
their relationships.  This is an evolving area of Searchfox, and is subject to
change.  This expected change also informs the design of the record format,
which is just a wrapper around an opaque JSON structure as far as `analysis.rs`
is concerned.  (All other record types are flat with a well known set of
keys/values.)

We emit structured records at the point of their definition.  Structured
records reference other structured records by their searchfox symbol identifier.
A structured record itself won't embed child types (even if they're not visible
outside the type) but instead reference them (which may involve searchfox
generating identifiers that have no meaning outside of searchfox, like is done
for locals).

Because of the realities of compilation, we know a parent class won't
necessarily know all of its subclasses, so all type references in emitted
records will generally only be upwards/sideways, never downwards.  We depend on
cross-referencing for determining sets of children/etc.

So for a class, we might expect the structured record in the analysis file to
contain:
- A list of its known super-classes.
- A list of its fields and members.

But it would not contain:
- A list of its known sub-classes.  This will be determined by
  cross-referencing.

#### Bytes and CharUnits

Clang defines a "Character Units" type
[`CharUnits`](https://clang.llvm.org/doxygen/classclang_1_1CharUnits.html#details)
that basically means bytes.  Searchfox just calls them bytes and assumes an
8-bit byte because strings are such a large part of the Firefox codebase that
calling bytes "char units" just adds terrifying confusion.

#### Formal Hierarchy:

Raw record info.  These are attributes that will be found in the analysis files.
- `pretty`: The pretty name/identifier for this structured symbol info.
- `sym`: The searchfox symbol for this symbol.
- `kind`: A string with one of the following values:
  - `enum`: TODO: I don't know that this actually is a thing we emit yet?
  - `class`
  - `struct`
  - `union`
  - `method`: It's a method on a class/struct.  It will have `overrides`.
  - `function`: A boring function.  No `overrides`.
  - `field`: A member of a class/struct.  Right now the field record has minimal
    info with the intent being that the data canonically lives on the parent
    symbol and this just provides the `parentsym` necessary to get to that info.
    But this potentially needs more thought.  TODO: Think more on this!
  - `ipc`: An IPC function where there's a send method and a recv method.  The
    send method will be associated via `srcsym` and the recv method via
    `targetsym`.  TODO: clarify what happens for the IPC interface.
- `parentsym`: For methods and fields, the symbol of the record to which they
  belong.  The current intent is that this is not populated for namespace
  purposes.  (That is, for a class "Bar" in namespace "foo" with pretty name
  "foo::Bar", "Bar" would not have a parentsym.)  The rationale is that we
  expect there to be a ton of stuff in any given namespace and we already have
  means of looking up the contents of a namespace via the `identifiers` table.
  This may want to evolve in the future, however.  Note that this attribute is
  not currently used for any cross-referencing, it's just meta.  (And note that
  target records' `contextsym` should frequently be the same when it's not just
  a namespace.)
- `srcsym`: For "ipc" calls, the symbol that corresponds to the send method.
- `targetsym`: For "ipc" calls, the symbol that corresponds to the recv method.
- `implKind`: Assume to be "impl" if not present.  Reasonable values:
  - `idl`: This is the semantic definition in XPIDL, IPDL, WebIDL, etc.
  - `binding`: Ex: WebIDL binding.
  - `impl`: By default, most things will be "impl".  But when WebIDL/etc. are
    involved this will be the actual implementation.
- `sizeBytes`: Size in bytes.  Not present for method/function.
- `supers`: For class-like symbols, an array of:
  - `pretty`: The pretty name/identifier for this super.
  - `sym`: The searchfox symbol for this super.
  - `props`: An array of strings whose presence indicates a semantic attribute:
    - `virtual`: It's a virtual base class if present.
- `methods`: For class-like symbols, an array of:
  - `pretty`: The pretty name/identifier for this method.
  - `sym`: The searchfox symbol for this method.  
  - `props`: An array of strings whose presence indicates a semantic attribute:
    - `static`: It's a static method (implies not "instance").
    - `instance`: It's a method on the instance (implies not "static").
    - `virtual`: It's a virtual method.
    - `user`: It's user-provided.
    - `defaulted`: It's defaulted per C++0x, AKA someone did `= default`.
    - `deleted`: It's deleted per C++0x, AKA someone did `= delete`.
    - `constexpr`: It's marked (C++11) constexpr!
- `fields`: For data-structure-like symbols, an array of:
  - `pretty`: The pretty name/identifier for this field.
  - `sym`: The searchfox symbol for this field.
  - `type`: Compiler's string representation of the type.  This is still
    somewhat experimental and the same thing we emit for source records.
  - `typesym`: The searchfox symbol for the type of this field.
  - `offsetBytes`: Byte offset of the field within this immediate structure.
  - `bitPositions`: Only present in bit-fields.  Object with the following
    properties whose names are derived from the AST dumper:
    - `begin`
    - `width`
  - `sizeBytes`: Only present in non-bit-fields.  The size of the fieldin bytes.
- `overrides`: For methods, an array of method signatures that are overridden.
  - `pretty`: The pretty name/identifier for the referenced method.
  - `sym`: The searchfox symbol for the referenced method.
- `props`: For methods, an array of strings whose presence indicates a semantic
  attribute.  These are the same as the props under a class-like symbol's
  `methods` array.
  - `static`: It's a static method (implies not "instance").
  - `instance`: It's a method on the instance (implies not "static").
  - `virtual`: It's a virtual method.
  - `user`: It's user-provided.
  - `defaulted`: It's defaulted per C++0x, AKA someone did `= default`.
  - `deleted`: It's deleted per C++0x, AKA someone did `= delete`.
  - `constexpr`: It's marked (C++11) constexpr!

Attributes added/updated by cross-referencing:
- `subclasses`: Derived from `supers`.
  - `pretty`
  - `sym`
- `overridenBy`: Derived from `overrides`.
  - `pretty`
  - `sym`
- `srcsym`: May exist in the record, also propagated from `idl` implKind
  records.
- `targetsym`: May exist in the record, also propagated from `idl` implKind
  records.
- `idlsym`: Linkaged established by `idl` implKind records from `srcsym` and
  `targetsym` symbols to the `idl` symbol.

Attributes optionally added by merging (see more on this below).  These will
only be present when the structured records differed between platforms.  If
every platform had the same structured record contents, that representation is
left as-is.
- `variants`: A list of structured record objects with the above
  (pre-cross-referencing) attributes.  Each of these records will also have a
  `platforms` Array of string platform names that had these attributes.
- `platforms`: Array of string platform names whose structured records were the
  same and chosen to be the canonical variant.

#### Merging of Structured Records

Note that there is also documentation in `merge-analyses.rs` alongside the code
implementing this logic which may be more straightforward to understand.

Merging is performed by:
- Hashing all of the structured records for a given symbol so that we can detect
  equivalent structured records.  (The records don't include the platform name
  at the time.)
- Checking if all the structured records were the same, and if so, just spitting
  out the singleton record as-is.
- If the records differed, which will frequently be the case at the time of
  having written this where we built 32-bit ARM builds in addition to the 64-bit
  builds for Windows/OS X/Linux, we arbitrarily pick a record to be the
  "canonical" structured record "variant".
  - Currently this is the last record we saw because this will never be the
    32-bit ARM record with our current config where arm gets listed first.
- The canonical variant ends up looking exactly like it would have without
  merging except we add a `variants` attribute which is a list of all of the
  other records we saw (consolidated by hashing).  Every variant (including the
  top-level canonical variant) gets a `platforms` attribute that is just an
  Array of Strings that are the platform names as used by searchfox.


### C++ inheritance

C++ inheritance is one of the most tricky issues to deal with in an
analysis like this. When the user clicks on a method call and
searches, should they find method definitions/calls for other classes?
If so, how should this work? Mozsearch is pretty naive here, but it
generally is good enough to find an over-approximation of the desired
results.

Mangled C++ names include the concrete class on which a method is
defined. So each symbol is automatically tagged with the name of the
class in which it's defined.

First let's consider method calls. Consider a method call
`obj->f`. The mangled name of `f` will include the class name of `obj`
(or, when it doesn't implement `f` itself, the first type above it in
the inheritance chain that does define `f`).  In this case, mozsearch
generates one source and one target record. The symbol in both records
is for `f`'s mangled name.

When a method `T::f` is defined, mozsearch finds all the methods that
`f` overrides in `T`'s direct and indirect superclasses (including via
multiple inheritance). It generates a single source record whose `sym`
property contains all of these symbols in a comma-delimited
list. Consequently, searching from this method definition will find
all calls to this implementation of `f`, either directly through `T`
or via supertypes of `T` (which may invoke this `f` through dynamic
dispatch).

We also generate one target record for each override of `f` in `T`'s
superclasses. This way, searching from any method call, either through
`T` or some superclass of `T` that might dispatch to this `f` via
dynamic dispatch, will find this `f`.

One flaw in this system is that it doesn't consider the full shape of
the inheritance tree. Consider this case:

```
class A {
  virtual void f() { return 1; }
};

class B : public A {};

class C : public A {
  virtual void f() override { return 2; } // Y
};

B* b;
b->f(); // Z
```

We will generate generate a source record at the line marked Z with
one symbol, for `A::f`. At line Y, we generate a target record for
`C::f` that has symbols for `C::f` and `A::f`. Consequently, searching
at line Z will find `C::f`. However, it's not actually possible for
`b->f()` to call `C::f`. A better technique would recognize this. That
is future work.

### Multiple passes over a single file

The clang plugin for indexing C++ files will typically analyze a given
header file many times. In some cases, the analysis records generated
for the header file will differ from one time to another. This happens
most often in the case of macros and templates. Different files that
include a header will instantiate a macro or template in different
ways, which may produce different analysis results.

To compensate for this issue, the clang plugin checks to see if an
analysis file already exists before writing one out. If one does
exist, it reads it in and merges its data with the new data. The merge
happens by combining the new and old records in a single vector,
sorting the vector (using an arbitrary sort order), and removing
duplicates. This new list of records is then written to disk. During
this time, the file is kept locked to avoid issues with parallel
compilation.
