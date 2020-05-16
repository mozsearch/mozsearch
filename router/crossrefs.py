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

                if key == None:
                    pos += linelen
                    key = line.strip()
                else:
                    value = line.strip()
                    s = "{},{}".format(pos, pos + len(value))
                    crossrefs[key] = s
                    key = None
                    pos += linelen

        repo_data[repo_name] = (mm, crossrefs)

def lookup(tree_name, symbols):
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
            results[k] = results.get(k, []) + result[k]

    return results
