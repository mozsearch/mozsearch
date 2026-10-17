# Functions for working with the `crossref` and `crossref-extra` cross-reference
# files documented in `crossref.md`.

import json
import sys
import mmap
import os.path
from logger import log

repo_data = {}
# Each tree's `linked_trees`: the trees which also have data for its symbols
# with the given prefixes (ex: `{"SPEC_": "wubspecs"}`), by prefix.
linked_trees = {}

def load(config):
    global repo_data

    for repo_name in config['trees']:
        log('Loading %s', repo_name)
        index_path = config['trees'][repo_name]['index_path']
        linked_trees[repo_name] = config['trees'][repo_name].get('linked_trees', {})

        inline_mm = None
        with open(os.path.join(index_path, 'crossref')) as f:
            try:
                inline_mm = mmap.mmap(f.fileno(), 0, prot=mmap.PROT_READ)
            except ValueError as e:
                log('Failed to mmap crossref file for %s: %s', repo_name, str(e))
                pass

        extra_mm = None
        with open(os.path.join(index_path, 'crossref-extra')) as f:
            try:
                extra_mm = mmap.mmap(f.fileno(), 0, prot=mmap.PROT_READ)
            except ValueError as e:
                log('Failed to mmap crossref file for %s: %s', repo_name, str(e))
                pass

        repo_data[repo_name] = (inline_mm, extra_mm)

NEWLINE_ORD = ord('\n')
ID_START_ORD = ord('!')
INLINE_STORED_STR = ':'
EXTERNALLY_STORED_STR = '@'

def get_id_line(mm, pos):
    '''
    Given a memory map and a position, expand from `pos` to find the identifier
    line (`!` prefixed) that covers the position.  Returns (the identifier,
    the offset of the `!` from the start of the identifier line, the offset of
    the newline ending the identifier line).

    `pos` is either inside an identifier line or a payload line that follows an
    identifier line, so we always walk backwards until we find an identifier.
    We should never need to walk forward (to find the start of the identifier
    line) because the result of any comparison should always tell the bisection
    to bisect in the positive direction (because the file is sorted), which
    should then find the subsequent record (if that's the one we're looking
    for, etc.).
    '''
    # We could the trailing newline as part of the record, so back up a char.
    if mm[pos] == NEWLINE_ORD:
        pos -= 1

    start = end = pos

    # Scan backwards until we hit the start of the file (where the first line
    # must be an identifier) or we hit a newline and the character following
    # the newline is the identifier prefix of `!`.
    while start > 0:
        if mm[start - 1] == NEWLINE_ORD:
            if mm[start] == ID_START_ORD:
                break
            else:
                # We're hitting a ":" and we need to reset end to this newline
                end = start - 1
                # and we want to keep going...
        start -= 1

    # Start should now be pointing at the `!` of the identifier line.

    size = mm.size()
    while end < size and mm[end] != NEWLINE_ORD:
        end += 1

    # end should now be pointing at the trailing newline.

    # Skip the leading `!` and decode the utf-8 encoded symbol
    line_sym = mm[start+1:end].decode('utf-8')
    return (line_sym, start, end)

def bisect_for_payload(mm, search_sym):
    '''
    Bisect the mmap to look for an exact symbol match `sym`, and returning the
    payload line which may be either inline JSON or external offsets to be
    retrieved from another map.
    '''

    first = 0
    count = mm.size()
    while count > 0:
        step = int(count / 2)
        pos = first + step

        (line_sym, line_start, line_end) = get_id_line(mm, pos)

        if line_sym == search_sym:
            ## Exact Match!
            mm.seek(line_end + 1)
            payload_line = mm.readline().decode('utf-8')
            return payload_line
        elif line_sym < search_sym:
            ## Bisect latter half
            # We might as well exclude the payload line we're skipping as well.
            # Because payload lines are intentionally limited during the
            # creation of `crossref`, we know this should fault an acceptable
            # number of pages which may have already been pre-fetched.
            next_newline = mm.find(b'\n', line_end + 1)
            if next_newline != -1:
                first = next_newline + 1
            else:
                # If there was no newline, then we're at the end and we might
                # as well stop.
                return None
            # Halve count and also subtract off the parts of the identifier line
            # and payload line we're skipping.  `first` is now effectively
            # `original_first + step + value_length` whereas `pos` is still
            # `original_first + step`.  So `first - pos` = `value_length`
            count -= step + (first - pos)
        else:
            ## Bisect first half
            # Halve count and subtract off the part of the identifier line that
            # we can eliminate from consideration.
            count = step - (pos - line_start)

    return None

# Fields of crossref records which the router doesn't use, and whose values
# aren't lists of results, which it assumes of all of them but 'meta' and
# 'callees' (ex: `lookup_merging`): symbols' history digests, for /query/'s
# recency facets (see tools/src/file_format/recency.rs), and classes' and
# subsystems' roll-ups, for block diagrams (see tools/src/file_format/rollup.rs).
UNUSED_FIELDS = ('recency', 'recency_from', 'callRollup', 'dataRollup')

def without_unused_fields(result):
    for field in UNUSED_FIELDS:
        result.pop(field, None)
    return result

def lookup_raw(tree_name, sym):
    '''
    Look up the given symbol from `crossref` and parse and return the resulting
    JSON as objects.
    '''
    (inline_mm, extra_mm) = repo_data[tree_name]

    if not inline_mm:
        return None

    payload = bisect_for_payload(inline_mm, sym)
    if not payload:
        return None

    if payload[0] == INLINE_STORED_STR:
        return without_unused_fields(json.loads(payload[1:]))
    elif payload[0] != EXTERNALLY_STORED_STR:
        # Fail if we're seeing something other than an external ref.
        return None

    (braceOffset, lengthWithNewline) = payload[1:].split(' ')
    (braceOffset, lengthWithNewline) = (int(braceOffset, 16), int(lengthWithNewline, 16))

    # exclude the newline
    data = extra_mm[braceOffset:(braceOffset + lengthWithNewline - 1)]

    result = json.loads(data)
    return without_unused_fields(result)

def is_path_results(value):
    return isinstance(value, list) and all(
        isinstance(p, dict) and 'path' in p and 'lines' in p for p in value)

def lookup_linked(tree_name, sym, result):
    '''
    Add the data of the trees linked to `tree_name` for `sym` (its
    `linked_trees`, by symbol prefix, ex: firefox-disco's `SPEC_` symbols'
    definitions in wubspecs, or wubspecs' citations in firefox-disco) to
    `result` (or return theirs, if `result` is None): their path results, in a
    `tree:<tree>` path kind, with their `tree` (so the search results have them
    in their own section, linking to that tree's files), and their `meta`, if
    `result` has none.
    '''
    for (prefix, other) in linked_trees.get(tree_name, {}).items():
        if not sym.startswith(prefix) or other not in repo_data:
            continue
        other_result = lookup_raw(other, sym)
        if not other_result:
            continue
        if result is None:
            result = {}
        for (k, v) in other_result.items():
            if k == 'meta':
                result.setdefault('meta', v)
                continue
            if not is_path_results(v):
                continue
            for pathr in v:
                pathr['path_kind'] = 'tree:' + other
                pathr['tree'] = other
            result[k] = result.get(k, []) + v
    return result

def lookup_merging(tree_name, symbols):
    '''
    Split `symbols` on commas, and lookup all of the requested symbols, merging
    their results.
    '''
    symbols = symbols.split(',')

    results = {}
    for symbol in symbols:
        result = lookup_linked(tree_name, symbol, lookup_raw(tree_name, symbol))
        if result is None:
            # we used to bail here, but it turns out this is actually a real
            # correctness problem when searching for XPIDL JS methods that don't
            # exist.
            continue

        for (k, v) in result.items():
            if k == 'callees':
                continue
            # expand_keys now expects aggregated meta, so wrap the meta obj.
            if k == 'meta':
                v = [v]
            results[k] = results.get(k, []) + v

    return results

def lookup_single_symbol(tree_name, symbol):
    '''
    Look up a single symbol (with its linked trees' data; see `lookup_linked`),
    returning its results dict if it existed or None if it didn't exist.
    '''
    return lookup_linked(tree_name, symbol, lookup_raw(tree_name, symbol))
