import json
import sys
import mmap
import os.path

repo_data = {}

def load(config):
    global repo_data

    for repo_name in config['repos']:
        print 'Loading', repo_name
        index_path = config['repos'][repo_name]['index_path']

        f = open(os.path.join(index_path, 'crossref'))
        mm = mmap.mmap(f.fileno(), 0, prot=mmap.PROT_READ)
        f.close()

        key = None
        pos = 0

        crossrefs = {}
        while True:
            line = mm.readline()
            if line == '':
                break

            if key == None:
                pos += len(line)
                key = line.strip()
            else:
                value = line.strip()
                s = "{},{}".format(pos, pos + len(value))
                crossrefs[key] = s
                key = None
                pos += len(line)

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
