# Analysis JSON

All data from the semantic analysis of a source file is dumped out to
JSON. For a file with path `${path}` from the repository root, the
analysis data is stored at `${index}/${tree_name}/analysis/${path}`.

The analysis data is broken into records. Each record corresponds to
an identifier in the original source code. Each line of the file
contains a record. (Technically the file is not JSON itself, but a
series of JSON objects delimited by line breaks.)

Analysis records are currently generated from:
* `js-analyze.js` for the JavaScript analysis.
* `idl-analyze.py` for IDL files.
* `clang-plugin/Indexer.cpp` for C++ files.

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

### Sources

A source record additionally contains a `syntax` property and a `pretty` property.

The `syntax` property describes how the identifier should be syntax highlighted.
It is a comma-delimited list of strings. Currently the only strings that have any
effect are:

* `def`, `decl`, and `idl` all cause the identifier to be shown in bold.
* `type` causes the identifier to be shown in a different color.

The `pretty` property is used to generate the context menu items for
the identifier. It should contain a human-readable description like
`constructor nsDocShell::nsDocShell` or `property
SessionStore.getTabState`.

### Targets

Target records additionally contain a `kind` property and a `pretty` property.

The `kind` property should be one of `use`, `def`, `decl`, `assign`,
or `idl`. This property determines whether the identifier will appear
under the "Uses", "Definitions", "Declarations", "Assignments", or
"IDL" category of the search results page.

The pretty property is also uses for the context menu. If a target
record is the only `def` target for a given symbol, then the context
menu for any source records with that symbol will contain a `Go to
${pretty}` entry, where `${pretty}` is the target's `pretty` property.

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
