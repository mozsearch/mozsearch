import sys
import pygit2
import os.path
import unicodedata
from datetime import datetime, tzinfo, timedelta
import time
import email
import email.utils
import subprocess
import urllib
import re
import cPickle

import cProfile

old_path = sys.argv[1]
new_path = sys.argv[2]

if len(sys.argv) == 4:
    hg_path = sys.argv[3]
else:
    hg_path = None

old_repo = pygit2.Repository(pygit2.discover_repository(old_path))
new_repo = pygit2.Repository(pygit2.discover_repository(new_path))

timers = {}

class Timer:
    def __init__(self, name):
        self.name = name

    def __enter__(self):
        self.t = time.time()
        return self

    def __exit__(self, type, value, traceback):
        t = time.time() - self.t
        timers[self.name] = timers.get(self.name, 0) + t
        if type:
            raise type, value, traceback

def print_timers():
    for (name, t) in timers.items():
        print name, t
        
class MyTimezone(tzinfo):
    def __init__(self, offset):
        self.offset = offset

    def utcoffset(self, dt):
        return timedelta(minutes=self.offset)

    def dst(self, dt):
        return timedelta(0)

def find_email(s):
    m = re.search(r'\b([a-zA-Z0-9._%+!-]+@[a-zA-Z0-9._%+!-]+)\b', s)
    if m:
        s = m.group(1)
    return s
    
def run_cmd(*args, **kwargs):
    p = subprocess.Popen(*args, **kwargs)
    (stdout, stderr) = p.communicate()

    return stdout

def splitlines(s):
    if s == '':
        return []

    lines = s.split('\n')
    if not lines[-1]:
        lines = lines[:-1]
    return lines

def sanitize(s):
    s2 = ''
    for c in s:
        if ord(c) < 128:
            s2 += c
    return s2.strip()

def get_tree_data(repo, tree, path):
    for elt in path:
        if elt in tree:
            item = tree[elt]
            tree = repo.get(item.id)
        else:
            return None
    return tree

def unmodified_lines(new_blob, old_blob):
    unchanged = []
    patch = old_repo.diff(a=old_blob, b=new_blob, flags=pygit2.GIT_DIFF_PATIENCE)

    if patch.delta.is_binary:
        return []

    latest_line = 0
    delta = 0

    for hunk in patch.hunks:
        for line in hunk.lines:
            if line.new_lineno != -1:
                for i in range(latest_line, line.new_lineno - 1):
                    unchanged.append((i, i + delta))
                latest_line = (line.new_lineno - 1) + 1

            if line.origin == '+':
                delta -= 1
            elif line.origin == '-':
                delta += 1
            elif line.origin == ' ':
                assert line.old_lineno == line.new_lineno + delta
                unchanged.append((line.new_lineno - 1, line.old_lineno - 1))

    count = len(splitlines(new_blob.data))
    for i in range(latest_line, count):
        unchanged.append((i, i + delta))

    return unchanged

def str_blame_info(rev, path, lineno, author):
    return '%s:%s:%d:%s' % (rev, path, lineno, author) 

def blame_info(commit, lineno):
    return str_blame_info(commit.id, '%', lineno,
                          unicodedata.normalize('NFKD', commit.author.name).encode('ascii', 'ignore'))

def fixup_blame(info, path, parent_path):
    if path == parent_path:
        return info

    (rev, fname, lineno, author) = info.split(':', 3)
    if fname == '%':
        fname = '/'.join(parent_path)
    return str_blame_info(rev, fname, int(lineno), author)

def blame_for_path(file_movement, commit, path):
    #print '  ', '/'.join(path)

    blob = get_tree_data(old_repo, commit.tree, path)
    lines = splitlines(blob.data)
    blame = [ blame_info(commit, i) for i in range(1, len(lines) + 1) ]

    for parent in reversed(commit.parents):
        parent_path = path
        if blob.id in file_movement.get(parent.id, {}):
            parent_path = file_movement[parent.id][blob.id].split('/')

        parent_blob = get_tree_data(old_repo, parent.tree, parent_path)
        if not parent_blob:
            continue

        parent_blame_commit = blame_map[parent.id]
        parent_blame_blob = get_tree_data(new_repo, parent_blame_commit.tree, parent_path)
        parent_blame = splitlines(parent_blame_blob.data)

        unmodified = unmodified_lines(blob, parent_blob)
        for (lineno, parent_lineno) in unmodified:
            blame[lineno] = fixup_blame(parent_blame[parent_lineno], path, parent_path)

    return blame

def build_blame_tree(builder, file_movement, commit, path):
    #print '  ', '/'.join(path)

    tree = get_tree_data(old_repo, commit.tree, path)
    parent_trees = [ get_tree_data(old_repo, c.tree, path) for c in commit.parents ]
    parent_blame_trees = [ get_tree_data(new_repo, blame_map[parent_id].tree, path) for parent_id in commit.parent_ids ]

    for entry in tree:
        for i in range(len(parent_trees)):
            parent_tree = parent_trees[i]
            if not parent_tree:
                continue
            parent_blame_tree = parent_blame_trees[i]
            if entry.name in parent_tree and parent_tree[entry.name].id == entry.id:
                builder.insert(entry.name, parent_blame_tree[entry.name].id, entry.filemode)
                break
        else:
            if entry.type == 'blob':
                blame = blame_for_path(file_movement, commit, path + [entry.name])
                blame = ''.join([ b + '\n' for b in blame ])
                blob_oid = new_repo.create_blob(blame)
                builder.insert(entry.name, blob_oid, entry.filemode)
            else:
                assert entry.type == 'tree'
                entry_builder = new_repo.TreeBuilder()
                build_blame_tree(entry_builder, file_movement, commit, path + [entry.name])
                builder.insert(entry.name, entry_builder.write(), entry.filemode)

def transform_revision(commit):
    new_parents = [ blame_map[parent_id].id for parent_id in commit.parent_ids ]

    file_movement = {}
    if len(commit.parents) == 1:
        parent = commit.parents[0]
        movement = {}

        with Timer("diff"):
            diff = old_repo.diff(a=parent.tree, b=commit.tree)
        with Timer("find_similar"):
            diff.find_similar(flags=pygit2.GIT_DIFF_FIND_COPIES, rename_limit=1000000)

        for patch in diff:
            delta = patch.delta
            if delta.old_file.path != delta.new_file.path:
                movement[delta.new_file.id] = delta.old_file.path

        file_movement[parent.id] = movement

    builder = new_repo.TreeBuilder()
    with Timer("build_blame_tree"):
        build_blame_tree(builder, file_movement, commit, [])
    tree = builder.write()

    reference = None
    try:
        new_repo.head
    except:
        reference = 'refs/heads/master'

    hg_id = git_to_hg_map.get(commit.id)
    if hg_id:
        msg = 'git %s\nhg %s\n' % (commit.id, hg_id)
    else:
        msg = 'git %s\n' % commit.id

    with Timer("Commit"):
        oid = new_repo.create_commit(reference,
                                     commit.author,
                                     commit.committer,
                                     msg,
                                     tree,
                                     new_parents)

        new_repo.head.set_target(oid)

        blame_map[commit.id] = new_repo.get(oid)
        print '  ->', oid

def index_mercurial(hg_path):
    if not hg_path:
        return {}

    #try:
    #    f = open('hg-map.pickle')
    #except:
    #    f = None

    #if f:
    #    return cPickle.load(f)

    hg_map = {}
    out = run_cmd(['hg', 'log', '-R', hg_path,
                   '--template', '{node}\n{date|hgdate}\n{author}\n{parents}\n{desc|firstline}\n', '--debug'],
                  stdout=subprocess.PIPE)
    lines = splitlines(out)
    i = 0
    while i < len(lines):
        node = lines[i]
        date = lines[i + 1]
        author = sanitize(lines[i + 2])
        parents = lines[i + 3]
        desc = sanitize(lines[i + 4])
        i += 5

        if author == 'Alexander Surkov <h<surkov.alexander@gmail.com>':
            author = 'hsurkov.alexander@gmail.com'
        if author[0] == '<' and author[-1] == '>':
            author = author[1:-1]
        if author[0] == '"' and author[-1] == '"':
            author = author[1:-1]
        author = find_email(author)

        (ts, offset) = date.split(' ')
        offset = int(offset)
        offset = (offset / 60) * 60
        date = '%s %d' % (ts, offset)

        parents = parents.split(' ')
        parents = [ x.split(':')[1] for x in parents if x and x != '-1:0000000000000000000000000000000000000000' ]

        hg_map[(frozenset(parents), date, author, desc)] = node

    f = open('hg-map.pickle', 'wb')
    cPickle.dump(hg_map, f)

    return hg_map

def index_existing():
    try:
        new_repo.head.target
    except:
        return {}

    blame_map = {}
    for commit in new_repo.walk(new_repo.head.target):
        orig = pygit2.Oid(hex=commit.message.split()[1])
        blame_map[orig] = commit

    return blame_map

def find_mercurial_commit(commit):
    if not hg_path or commit.id in git_to_hg_map:
        return

    if commit.author.email == commit.author.name or commit.author.email == 'none@none':
        commit_user = commit.author.name
    elif commit.author.name == '':
        commit_user = commit.author.email
    elif ' ext:' in commit.author.name:
        i = commit.author.name.index(' ext:')
        name = commit.author.name[:i]
        ext = urllib.unquote(commit.author.name[i + 6:-1])
        commit_user = '%s <%s>%s' % (name, commit.author.email, ext)
    else:
        commit_user = '%s <%s>' % (urllib.unquote(commit.author.name), commit.author.email)
    commit_user = find_email(commit_user)

    commit_desc = splitlines(commit.message)[0]
    commit_date = '%d %d' % (commit.author.time, -(commit.author.offset * 60))

    parents = [ git_to_hg_map.get(parent.id) for parent in commit.parents ]

    key = (frozenset(parents), commit_date, sanitize(commit_user), sanitize(commit_desc))
    if key in hg_map:
        git_to_hg_map[commit.id] = hg_map[key]

print 'Indexing mercurial...'
hg_map = index_mercurial(hg_path)

print 'Computing existing blame map...'
blame_map = index_existing()
        
git_to_hg_map = {pygit2.Oid(hex='05e5d33a570d48aed58b2d38f5dfc0a7870ff8d3'):
                 '9b2a99adc05e53cd4010de512f50118594756650',
                 pygit2.Oid(hex='127762a4a507dd361c98e30e9c6a261f09083786'):
                 '39ed20d043d6f3dfbc764307aede7366a9c037d6'}
        
def transform():
    index = 0
    count = 0
    for commit in old_repo.walk(old_repo.head.target, pygit2.GIT_SORT_TOPOLOGICAL | pygit2.GIT_SORT_REVERSE):
        index += 1

        find_mercurial_commit(commit)

        if commit.id not in blame_map:
            print 'Transforming', commit.id, '(' + str(index) + ')', 'hg', git_to_hg_map.get(commit.id)

            transform_revision(commit)
            count += 1

            if count % 100 == 0:
                print_timers()

transform()
print_timers()
