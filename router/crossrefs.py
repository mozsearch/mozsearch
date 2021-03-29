from __future__ import absolute_import
import json
import sys
import mmap
import os.path
from logger import log

repo_data = {}

def load(config):
    global repo_data

    for repo_name in config['trees']:
        log('Loading %s', repo_name)
        index_path = config['trees'][repo_name]['index_path']

        mm = None
        with open(os.path.join(index_path, 'crossref')) as f:
            try:
                mm = mmap.mmap(f.fileno(), 0, prot=mmap.PROT_READ)
            except ValueError as e:
                log('Failed to mmap crossref file for %s: %s', repo_name, str(e))
                pass

        crossrefs = {}
        if mm:
            key = None
            pos = 0
            while True:
                line = mm.readline()
                linelen = len(line)
                if linelen == 0:
                    break

                # Crossrefs file is written by Rust in utf-8
                line = line.decode('utf-8').strip()
                if key == None:
                    pos += linelen
                    key = line
                else:
                    value = line
                    s = "{},{}".format(pos, pos + linelen)
                    crossrefs[key] = s
                    key = None
                    pos += linelen

        repo_data[repo_name] = (mm, crossrefs)

def lookup_merging(tree_name, symbols):
    '''
    Split `symbols` on commas, and lookup all of the requested symbols, merging
    their results.
    '''
    symbols = symbols.split(',')

    (mm, crossrefs) = repo_data[tree_name]

    results = {}
    for symbol in symbols:
        s = crossrefs.get(symbol)
        if s == None:
            return {}

        (startPos, endPos) = s.split(',')
        (startPos, endPos) = (int(startPos), int(endPos))

        data = mm[startPos:endPos]
        result = json.loads(data)

        for (k, v) in result.items():
            if k == 'meta' or k == 'consumes':
                continue
            results[k] = results.get(k, []) + result[k]

    return results

def lookup_single_symbol(tree_name, symbol):
    '''
    Look up a single symbol, returning its results dict if it existed or None
    if it didn't exist.
    '''
    (mm, crossrefs) = repo_data[tree_name]

    s = crossrefs.get(symbol)
    if s == None:
        return None

    (startPos, endPos) = s.split(',')
    (startPos, endPos) = (int(startPos), int(endPos))

    data = mm[startPos:endPos]
    result = json.loads(data)

    return result
