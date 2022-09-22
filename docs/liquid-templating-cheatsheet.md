# Liquid Templating Cheatsheet

Mozsearch uses the [liquid crate](https://crates.io/crates/liquid) which is a
rust implementation of the liquid templating language.  Documentation for the
[JS](https://liquidjs.com/tutorials/intro-to-liquid.html) and the canonical
[Ruby](https://shopify.github.io/liquid/) implementations are available and are
quite good, but note that [liquid-rust](https://github.com/cobalt-org/liquid-rust)
currently does not implement all of the blocks/tags/filters that are documented
at those implementations.  liquid was chosen from the limited pool of rust
templating engines that also had JS implementations available so that we could
use the same logic on both the server and client if needed.

## Liquid Core Semantics

### Blocks, Tags, Outputs, and Filters

Tags look like `{% foo %}`.  Outputs look like `{{ some_var }}`.  liquid-rust
adds an additional concept of "blocks" which are tags which can have other tags
conceptually nested inside them, like `{% for %}` which gets paired with
`{% endfor %}`.   Outputs can also include filters which use a pipe-like syntax
that looks like `{{ my_mixed_case_string || downcase }}`.

### Truthy / Falsy / Empty / Blank

Under the canonical ruby semantics, the only things that are falsy are `false`
and `nil`.  Empty strings, empty arrays, and empty objects are all truthy!  If
you would like sane results, you need to compare against the magic literals of
`empty` and/or `blank`.

There is logic [in the core method value_eq](https://github.com/cobalt-org/liquid-rust/blob/7b767eea877990ae96a6761b9ed74db8baab8f9e/crates/core/src/model/value/view.rs#L287-L291)
which checks if the LHS or RHS is an explicit ["state"](https://github.com/cobalt-org/liquid-rust/blob/7b767eea877990ae96a6761b9ed74db8baab8f9e/crates/core/src/model/value/state.rs)
and in that case uses the "query_state" method to check the semantics of the
value for the given state; for example [the str ValueView has this logic](https://github.com/cobalt-org/liquid-rust/blob/7b767eea877990ae96a6761b9ed74db8baab8f9e/crates/core/src/model/scalar/mod.rs#L553-L560).  This provides the mechanism for "truthy" (used by the
[if block's existence condition](https://github.com/cobalt-org/liquid-rust/blob/30ad5c4e3f84f918c1be46215187bcbb5ebde37d/crates/lib/src/stdlib/blocks/if_block.rs#L341))
as well as for the "[default](https://liquidjs.com/filters/default.html)" filter's
logic which checks for falsy or blank, not just falsy.

The difference between `empty` and `blank` is that `blank` will trim a string before
checking if it's empty.  So `"" == empty` and `"" == blank` but for `" "` we find
that `" " != empty` and `" " == blank`.

#### Is a string blank / empty?

Want to check if a string is exactly `""`?  Then use `my_var == empty`.  Do you
also want to act like it's empty if it's only whitespace?  Then use
`my_var == blank`.

#### Is an array empty?

Use `empty`.  Do `my_array_val == empty`.

### Is an object empty?

Want to check if an object dictionary has no keys/values?  Use `empty`.  Do
`my_obj == empty`.  Be aware that if you try and access a nonexistent property
of an object you will get an IndexError, so in many cases you actually want to
be using the `contains` operator instead; see the next section for more info.

### Does an object have a specific property?

liquid-rust currently does not have support for "EmptyDrop" as documented at
https://shopify.github.io/liquid/basics/types/ so if you try and access a
property that does not exist, you will get an IndexError.  If EmptyDrop support
existed, in theory it would just return something that is equal to empty, but
the ruby implementation doesn't seem to explicitly test for this behavior, so it
may just be an implementation artifact.

In any event, you can use the "contains" operator to do `my_obj contains "key"`
to check if there's a key property.  The canonical documentation for contains at
https://shopify.github.io/liquid/basics/operators/ is a little confusing because
the way it discusses object makes it sound like you can't do this, but I think
it's just saying it can only test string equality / check for keys and is not
capable of doing structural equality tests.

### Other Conditional Logic

#### There's a "contains" operator

In a conditional you can do things like `my_array contains "foo"` in an if tag
like `{% if my_array contains "foo" %}`.  As noted above about objects, this
also works for objects, so `my_obj contains "foo"` should return true for an
underlying object that looks like `{ "foo": ... }` in JSON.

#### There are "and" and "or" operators that operate right-to-left

You can't use parentheses or otherwise nest things.  It's like the rightmost
pair is fully nested in parentheses.  Don't even think about asking about
short-circuiting.

### Types

#### Arrays are zero-based and indexed with square brackets

`my_array_val[0]` is the first item in the array.  Usually you would use a
[for](https://liquidjs.com/tags/for.html) block tag, not directly subscript
things.  Subscripting makes sense for tuple types, such as when iterating over
an object/map.

### Whitespace Control

Putting a `-` character on the inside of a tag indicates to strip the whitespace
on that side of the tag.  So `{{-` /`-}}` can be used instead of `{{`/`}}` and
`{%-`/`-%}` can be used instead of `{%`/`%}`.  Note that you can make the
decision independently for the opening and closing tags.

## Liquid-Rust Supported Tags / Filters

### Built-in

This list is derived from examining the source's [blocks](https://github.com/cobalt-org/liquid-rust/blob/master/crates/lib/src/stdlib/blocks/mod.rs),
[tags](https://github.com/cobalt-org/liquid-rust/blob/master/crates/lib/src/stdlib/tags/mod.rs),
and [filters](https://github.com/cobalt-org/liquid-rust/blob/master/crates/lib/src/stdlib/filters/mod.rs) modules for the stdlib.

- Blocks:
  - capture
  - case
  - comment
  - for
  - if
  - ifchanged (only outputs its rendered contents if they've changed since the
    last outputted value, starting from the base case)
  - raw
  - tablerow
  - unless
- Tags
  - assign
  - break
  - continue
  - cycle
  - decrement
  - include (note: "render" is not supported right now!!)
  - increment
- Filters
  - abs
  - append
  - at_least
  - at_most
  - capitalize
  - ceil
  - compact
  - concat
  - date
  - default
  - divided_by
  - downcase
  - escape
  - escape_once
  - first
  - floor
  - join
  - last
  - lstrip
  - map
  - minus
  - modulo
  - newline_to_br
  - plus
  - prepend
  - reverse
  - remove
  - remove_first
  - replace
  - replace_first
  - round
  - rstrip
  - size: Number of elements in an array or object, length for strings.
  - slice
  - split
  - strip
  - strip_newlines
  - sort
  - sort_natural
  - strip_html
  - times
  - truncate
  - truncate_words
  - uniq
  - upcase
  - url_decode
  - url_encode
  - where

### Mozsearch additions

- Filters
  - compact_pathlike: Remove excess whitespace in a path-like string.  Compacts
    `" foo /  bar/ baz "` to `"foo/bar/baz"`.
  - ensure_bug_url: If we're given something that's clearly a link, pass it
    through as-is, but if it's not a bug, format it into a proper bug tracker
    link, which by default is (or may still be hardcoded to be) BMO.
  - fileext: Extracts the file extension from a path string, defaulting to the
    empty string if there is no file extension.  (Note that this does not use
    the "default" mechanism!)
  - json: Render the given value to JSON
  - `strip_prefix_or_empty`: Takes an argument which is a prefix to attempt to
    remove.  If the string started with the prefix, the prefix-stripped string
    is returned.  If the string did not start with the prefix, an empty string
    is returned.  We probably could also have just returned the false value but
    idiomatically it's probably better to have explicit checks against `empty`
    everywhere to reduce confusion.
