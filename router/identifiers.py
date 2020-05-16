from __future__ import absolute_import
from __future__ import print_function
import json
import sys
import mmap
import os.path
from logger import log

repo_data = {}

def load(config):
    global repo_data

    for repo_name in config['trees']:
        log('Loading identifiers for %s', repo_name)
        index_path = config['trees'][repo_name]['index_path']

        mm = None
        with open(os.path.join(index_path, 'identifiers')) as f:
            try:
                mm = mmap.mmap(f.fileno(), 0, prot=mmap.PROT_READ)
            except ValueError as e:
                log('Failed to mmap identifiers file for %s: %s', repo_name, str(e))
                pass

        repo_data[repo_name] = mm

def get_line(mm, pos):
    if mm[pos] == '\n':
        pos -= 1

    start = end = pos

    while start >= 0 and mm[start] != '\n':
        start -= 1
    start += 1

    size = mm.size()
    while end < size and mm[end] != '\n':
        end += 1

    return mm[start:end]

def bisect(mm, needle, upper_bound):
    needle = needle.upper()

    first = 0
    count = mm.size()
    while count > 0:
        step = int(count / 2)
        pos = first + step

        line = get_line(mm, pos).upper()
        if line < needle or (upper_bound and line == needle):
            first = pos + 1
            count -= step + 1
        else:
            count = step

    return first

def lookup(tree_name, needle, complete, fold_case):
    mm = repo_data[tree_name]

    if not mm:
        return []

    first = bisect(mm, needle, False)
    last = bisect(mm, needle + '~', True)

    result = []
    mm.seek(first)
    while mm.tell() < last:
        line = mm.readline().strip()
        pieces = line.split(' ')
        suffix = pieces[0][len(needle):]
        if ':' in suffix or '.' in suffix or (complete and suffix):
            continue
        if not fold_case and not pieces[0].startswith(needle):
            continue
        result.append(pieces[0:2])

    return result

if __name__ == '__main__':
    load(json.load(open(sys.argv[1])))
    print(lookup(sys.argv[2], sys.argv[3]))
