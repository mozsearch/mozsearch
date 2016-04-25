import sys
import pygit2
import os.path

tree_root = sys.argv[1]
cset = sys.argv[2]
path = sys.argv[3]

repo = pygit2.Repository(pygit2.discover_repository(tree_root))

def splitlines(s):
    if s == '':
        return []

    lines = s.split('\n')
    if not lines[-1]:
        lines = lines[:-1]
    return lines

def get_tree_data(tree, path):
    elts = path.split(os.path.sep)
    for elt in elts:
        if elt in tree:
            item = tree[elt]
            tree = repo.get(item.id)
        else:
            return None
    return tree

# Will return a series of lines. Lines will have numbers and they will be either
# additions, deletions, or unchanged. For a merge commit, I guess I can try to
# merge everything together. Not sure how to do that yet.
# For unchanged and deleted lines, I will need blame information.

# Merging: an unchanged line becomes an addition if it's not an unchanged
# line in every parent. It also becomes a subtraction for every parent where
# it was an unchanged line. Otherwise nothing changes. Subtractions are merged
# in naturally. Basically I treat additions/unchanged lines as the "index" and
# subtractions are interstitial. So generate_diff_single would be better off
# returning the complete text with subtractions "in between" and a flag saying
# whether a "new line" existed in the old file or not. So each line will have
# a list of subtractions that will go before it.
#   [Line]
#   Line ::= (Contents, [Previous])
#   Previous ::= (Rev, Contents, BlameOrNone)

def generate_diff_single(path, commit, parent):
    new_blob = get_tree_data(commit.tree, path)
    old_blob = get_tree_data(parent.tree, path)

    if not new_blob:
        lines = old_blob.data.splitlines(True)
        lines = [('', [ (l, None) for l in lines ])]
    else:
        lines = new_blob.data.splitlines(True)
        lines = [ (line, []) for line in lines ]
        patch = repo.diff(a=old_blob, b=new_blob, flags=pygit2.GIT_DIFF_PATIENCE)

        latest_line = 0

        for hunk in patch.hunks:
            for line in hunk.lines:
                #print line.origin, line.new_lineno, latest_line, line.content,
                if line.origin == '-':
                    lines[latest_line][1].append((line.content, None))
                else:
                    for i in range(latest_line + 1, line.new_lineno):
                        lines[i - 1][1].append((lines[i - 1][0], None))

                    if line.origin != '+':
                        lines[line.new_lineno - 1][1].append((line.content, None))
                    latest_line = line.new_lineno

    # FIXME: Do I have to finish from latest_line to end?

    blame = repo.blame(path, newest_commit=parent.id)
    (i, j) = (0, 0)
    for hunk in blame:
        lno = hunk.orig_start_line_number
        for x in range(hunk.lines_in_hunk):
            (line, old) = lines[i]
            if j == len(old):
                continue
            old[j] = (old[j][0], str(hunk.final_commit_id) + '#' + str(lno + x))

            j += 1
            while j == len(old) and i < len(lines) - 1:
                j = 0
                i += 1
                (line, old) = lines[i]

    return lines
    
def generate_diff(path, commit):
    if not commit.parents:
        blob = get_tree_data(commit.tree, path)
        lines = blob.data.splitlines(True)
        return [ (i+1, line, None, None, None) for (i, line) in enumerate(lines) ]

    lines = []
    for parent in commit.parents:
        lines2 = generate_diff_single(path, commit, parent)
        lno = 1
        for (rev, old_data, new_data) in lines2:
            lines.append((lno, new_data, parent.id, old_data, rev))
            if new_data != None:
                lno += 1

    return sorted(lines)

def output_html(lines):
    print '<table>'
    print '<tr>'

    output = []
    for (i, (line, old)) in enumerate(lines):
        unchanged = False
        blamerev = None
        if len(old) and old[-1][0] == line:
            blamerev = old[-1][1]
            old = old[:-1]
            unchanged = True

        for (oldline, blame) in old:
            output.append((None, blame, '-', oldline[:-1]))

        if unchanged:
            output.append((i + 1, blamerev, ' ', line[:-1]))
        else:
            output.append((i + 1, None, '+', line[:-1]))

    def color(origin):
        if origin == '+':
            return 'blue'
        elif origin == '-':
            return 'red'
        else:
            return 'black'

    print '<td><pre>'
    for (lno, blame, origin, line) in output:
        if lno:
            print '<code>', lno, '</code>'
        else:
            print ''
    print '</pre></td>'

    print '<td><pre>'
    for (lno, blame, origin, line) in output:
        if blame:
            print blame
        else:
            print ''
    print '</pre></td>'
    
    print '<td><pre>'
    for (lno, blame, origin, line) in output:
        print '<code style="color: %s;">%s</code>' % (color(origin), origin)
    print '</pre></td>'

    print '<td><pre>'
    for (lno, blame, origin, line) in output:
        print '<code style="color: %s;">%s</code>' % (color(origin), line)
    print '</pre></td>'

    print '</table>'
    
commit = repo.get(cset)
lines = generate_diff_single(path, commit, commit.parents[0])

#for (i, l) in enumerate(lines):
#    print i + 1, l

output_html(lines)
