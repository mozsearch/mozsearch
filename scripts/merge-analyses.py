#!/usr/bin/env python

# This script merges the analysis data in the files provided as
# arguments, and prints the merged analysis data to stdout.
# The "target" data lines from the input files are copied to
# the output as-is. The "source" data lines are merged such
# that the syntax and symbol properties are unioned across
# all input lines that have a matching (loc, pretty) tuple.
# This ensures that for a given identifier, only a single context
# menu item will be displayed for a given "pretty" representation,
# and that context menu will link to all the symbols from all
# the input files that match that.

# Note that this code intentionally preserves the order of stuff
# within each line, as much as possible. However, the lines
# themselves get reordered, but the output can be piped through
# `sort` as needed. Maintaining the order of stuff within the
# line is important as reordering things can result in identical
# lines getting sorted differently. e.g.
#   { loc: foo, pretty: bar }
# and
#   { pretty: bar, loc: foo }
# are semantically identical but would not get deduplicated by
# piping through `sort` and `uniq`.

from collections import OrderedDict
import json
import sys

if len(sys.argv) == 1:
    sys.stderr.write("Usage: merge-analyses.py <filename> [<filename> ...]\n")
    sys.stderr.write("  This script will merge the analysis data from the given files\n")
    sys.stderr.write("  and print it to stdout\n")
    sys.exit(1)

sourcemap = {}

def SplitToOrderedSet(comma_sep_str):
    return OrderedDict([(item, None) for item in comma_sep_str.split(',')])

for filename in sys.argv[1:]:
    with open(filename) as filehandle:
        for line in filehandle.readlines():
            # parse the JSON into an OrderedDict to maintain order of properties
            entry = json.loads(line, object_pairs_hook=OrderedDict)

            if "source" not in entry:
                print(line.strip())
                continue

            for prop in entry:
                if prop not in ["source", "loc", "pretty", "syntax", "sym", "no_crossref"]:
                    sys.stderr.write("WARNING: Unexpected property %s found in source line in analysis file %s\n" % (prop, filename))

            key = (entry["loc"], entry["pretty"])
            if key in sourcemap:
                # We already encountered a line with this (loc,pretty) tuple,
                # so merge the syntax and sym properties. Again use an
                # OrderedDict (with None values, so it is effectively an
                # ordered set) to avoid reordering the existing tokens.
                oldEntry = sourcemap[key]
                syntaxes = SplitToOrderedSet(oldEntry["syntax"])
                syntaxes.update(SplitToOrderedSet(entry["syntax"]))
                oldEntry["syntax"] = ','.join(i for i in syntaxes)
                symbols = SplitToOrderedSet(oldEntry["sym"])
                symbols.update(SplitToOrderedSet(entry["sym"]))
                oldEntry["sym"] = ','.join(i for i in symbols)
                sourcemap[key] = oldEntry
            else:
                # Haven't encountered this before, so add it to the map
                sourcemap[key] = entry

for key in sourcemap:
    print(json.dumps(sourcemap[key], separators=(',', ':')))
