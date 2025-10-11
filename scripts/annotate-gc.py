#!/usr/bin/env python3

import sys
import json
import os


def load_functions(gc_functions_path, all_functions_path):
    """Load gcFunctions.txt and allFunctions.txt from hazard analysis
    and create a map from the function symbol to GC info.
    GC functions have the path to the GC as a value, and
    GC-free functions have None as a value."""
    functions = {}

    with open(all_functions_path) as f:
        for line in f:
            sym = line.rstrip().split("$", 1)[0]
            functions[sym] = None

    with open(gc_functions_path) as f:
        current = None
        path = []

        for line in f:
            line = line.rstrip()
            if line.startswith("GC Function: "):
                if current is not None:
                    functions[current] = "\n".join(path)

                current = line[13:].split("$", 1)[0]
                path = []
            elif line.startswith(" "):
                path.append(line.lstrip())

        if current is not None:
            functions[current] = "\n".join(path)

    return functions


def get_syms(datum):
    """Return all symbol variants in the structured record."""
    syms = [datum["sym"]]
    if "variants" in datum:
        for v in datum["variants"]:
            syms.append(v["sym"])
    return syms


def annotate_gc(analysis_dir, path, functions):
    """Annotate structured analysis record with GC info."""
    annotated = False
    lines = []

    fullpath = os.path.join(analysis_dir, path)

    with open(fullpath, "r") as f:
        for line in f:
            line = line.rstrip()

            # Filter out definitely non-target lines.
            if "structured" not in line:
                lines.append(line)
                continue

            datum = json.loads(line)
            syms = get_syms(datum)

            # Filter out unknown symbols.
            # NOTE: The hazard analysis is done on linux64,
            #       but the merged structurd record's main variant isn't
            #       guaranteed to be linux64.
            #       Check all symbols.
            found = False
            for sym in syms:
                if sym in functions:
                    found = True
                    break

            if not found:
                lines.append(line)
                continue

            maybe_path = functions[sym]

            if maybe_path is None:
                datum["canGC"] = False
            else:
                datum["canGC"] = True
                datum["gcPath"] = maybe_path

            annotated = True

            lines.append(json.dumps(datum))

    if not annotated:
        return

    with open(fullpath, "w") as f:
        for line in lines:
            print(line, file=f)


analysis_dir = sys.argv[1]
gc_functions_path = sys.argv[2]
all_functions_path = sys.argv[3]

if not os.path.exists(gc_functions_path):
    sys.exit(0)

if not os.path.exists(all_functions_path):
    sys.exit(0)

functions = load_functions(gc_functions_path, all_functions_path)

for path in sys.stdin:
    path = path.rstrip()
    if not path:
        continue

    annotate_gc(analysis_dir, path, functions)

