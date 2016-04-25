import sys
import pygit2
import os.path

repo_path = sys.argv[1]
rev = sys.argv[2]
path = sys.argv[3]

path = path.split('/')

repo = pygit2.Repository(pygit2.discover_repository(repo_path))

def get_tree_data(tree, path):
    for elt in path:
        if elt in tree:
            item = tree[elt]
            tree = repo.get(item.id)
        else:
            return None
    return tree

commit2 = repo.get(rev)
commit1 = commit2.parents[0]

blob1 = get_tree_data(commit1.tree, path)
blob2 = get_tree_data(commit2.tree, path)

patch = repo.diff(a=blob1, b=blob2)
for hunk in patch.hunks:
    for line in hunk.lines:
        print line.origin, line.old_lineno, line.new_lineno, line.content,
    print '---'
