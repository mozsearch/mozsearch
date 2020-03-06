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
    hg_map_file = sys.argv[3]
else:
    hg_map_file = None

_git_to_hg_map = None
use_cinnabar = bool(os.environ.get("CINNABAR"))

def to_hg(git_oid):
    global _git_to_hg_map

    if _git_to_hg_map is None and hg_map_file:
        _git_to_hg_map = {}
        print('Indexing mercurial map file...')
        with open(hg_map_file) as f:
            for line in f.readlines():
                (git_rev, hg_rev) = line.strip().split()
                _git_to_hg_map[pygit2.Oid(hex=git_rev)] = hg_rev

    if _git_to_hg_map is not None:
        ref = _git_to_hg_map.get(git_oid)
        if ref:
            return ref

    if use_cinnabar:
        # TODO: Maybe there's a way to not spawn processes here hooking into cinnabar's python code?
        # But it's not clear this is necessarily a perf issue...
        ref = subprocess.check_output(["git", "cinnabar", "git2hg", str(git_oid)], cwd=old_path).strip()
        if len(ref) != 0 and ref != ("0" * len(ref)):
            return ref

    return None

old_repo = pygit2.Repository(pygit2.discover_repository(old_path))
new_repo = pygit2.Repository(pygit2.discover_repository(new_path))

# Set BLAME_REF to something like 'refs/heads/beta' to transform
# commits from the 'beta' branch in the source repo to the 'beta'
# branch in the destination repo. Obviously the source repo must
# have a beta branch already defined; if the destination repo doesn't
# have one, one will be created. But be careful! If the destination
# repo doesn't have the branch, none of the existing commits in the
# repo will be read, so commits that already exist there (because they
# are common ancestors of the new branch and some other pre-existing
# branch, for example) will be "converted" again which can lead to
# duplicated commits at worst or wasted work at best. It is therefore
# advisable to ensure the destination repo also has the branch already
# defined, and points to a commit that is a descendant of all the
# common ancestors, so that those ancestors get read before any new
# commits are created. The only case where it makes sense to not have
# the branch in the destination repo, is if the destination repo is
# brand new and has nothing in it.
blame_ref = os.environ.get('BLAME_REF') or 'HEAD'

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
        # Tree should never be None, but Servo has a broken submodule entry in
        # commit b2a5225831a8eee3ff596dce2be8dc08df4300a0 pointing to a wpt
        # submodule, but without information about it in .gitmodules.
        if not tree or elt not in tree:
            return None

        item = tree[elt]
        tree = repo.get(item.id)

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
        if not parent_blob or not isinstance(parent_blob, pygit2.Blob):
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
            elif entry.type == 'commit':
                # This is a submodule, just treat it as an empty dir. We could
                # probably also skip over it entirely.
                entry_builder = new_repo.TreeBuilder()
                builder.insert(entry.name, entry_builder.write(), entry.filemode)
            else:
                assert entry.type == 'tree', "Unexpected type %s" % entry.type
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
    ref = None
    try:
        ref = new_repo.lookup_reference(blame_ref).resolve()
    except:
        # ref doesn't exist yet, so let's create it
        reference = blame_ref

    hg_id = to_hg(commit.id)
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

        if ref is not None:
            ref.set_target(oid)

        blame_map[commit.id] = new_repo.get(oid)
        print '  ->', oid


def index_existing():
    ref = None
    try:
        ref = new_repo.lookup_reference(blame_ref).resolve()
    except:
        # ref doesn't exist yet, so nothing to index
        return {}

    blame_map = {}
    for commit in new_repo.walk(ref.target):
        orig = pygit2.Oid(hex=commit.message.split()[1])
        blame_map[orig] = commit

    return blame_map

print 'Computing existing blame map...'
blame_map = index_existing()

def transform():
    index = 0
    count = 0
    ref = old_repo.lookup_reference(blame_ref).resolve()
    print 'Starting from ref ', blame_ref, ' = ', ref.target
    for commit in old_repo.walk(ref.target, pygit2.GIT_SORT_TOPOLOGICAL | pygit2.GIT_SORT_REVERSE):
        index += 1

        if commit.id not in blame_map:
            print 'Transforming', commit.id, '(' + str(index) + ')', 'hg', to_hg(commit.id)

            transform_revision(commit)
            count += 1

            if count % 100 == 0:
                print_timers()

            if count % 25000 == 0:
                run_cmd(['git', 'gc'], cwd=new_path)

transform()
print_timers()
