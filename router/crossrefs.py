import json
import sys
import mmap
import os.path

f = None
mm = None
crossrefs = {}

def load(config):
    global f, mm

    indexPath = config['mozilla-central']['index_path']
    f = open(os.path.join(indexPath, 'crossref'))
    mm = mmap.mmap(f.fileno(), 0, prot=mmap.PROT_READ)
    f.close()

    key = None
    pos = 0

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

def lookup(symbols):
    symbols = symbols.split(',')

    results = {}
    for symbol in symbols:
        s = crossrefs.get(symbol)
        if s == None:
            return {}

        (startPos, endPos) = s.split(',')
        (startPos, endPos) = (int(startPos), int(endPos))

        data = mm[startPos:endPos]
        result = json.loads(data)
        f.close()

        for (k, v) in result.items():
            results[k] = results.get(k, []) + result[k]

    return results
