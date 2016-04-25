import SimpleHTTPServer
import SocketServer

import sys
import os.path
import subprocess
import pygit2
import cgi
import re
from datetime import datetime, tzinfo, timedelta

tree_root = sys.argv[1]
blame_root = sys.argv[2]

repo = pygit2.Repository(pygit2.discover_repository(tree_root))
blame_repo = pygit2.Repository(pygit2.discover_repository(blame_root))

class MyTimezone(tzinfo):
    def __init__(self, offset):
        self.offset = offset

    def utcoffset(self, dt):
        return timedelta(minutes=self.offset)

    def dst(self, dt):
        return timedelta(0)

def splitlines(s):
    if s == '':
        return []

    lines = s.split('\n')
    if not lines[-1]:
        lines = lines[:-1]
    return lines

def get_tree_data(repo, tree, path):
    elts = path.split(os.path.sep)
    for elt in elts:
        if elt in tree:
            item = tree[elt]
            tree = repo.get(item.id)
        else:
            return None
    return tree

def runCmd(*args, **kwargs):
    p = subprocess.Popen(*args, **kwargs)
    (stdout, stderr) = p.communicate()

    return stdout

def linkify(msg):
    return re.sub(r'\b([1-9][0-9]{4,9})\b',
                  r'<a href="https://bugzilla.mozilla.org/show_bug.cgi?id=\1">\1</a>',
                  msg)

def show_file(f, commit, path):
    print 'SHOW FILE', path

    parents = commit.parents

    difftxt = runCmd(['/usr/bin/git', 'diff-tree', '-p', '--cc', '--patience',
                      '--full-index', '--no-prefix', '-U100000',
                      str(commit.id), '--', path],
                     stdout=subprocess.PIPE, cwd=tree_root)

    lines = None
    if difftxt:
        lines = splitlines(difftxt)
        i = 0
        while i < len(lines) and not lines[i].startswith('@'):
            i += 1
        lines = lines[i + 1:]

    if not lines:
        blob = get_tree_data(repo, commit.tree, path)
        if not blob:
            print >>f, '<h1>File %s does not exist in commit %s!</h1>' % (path, commit.id)
            return

        lines = splitlines(blob.data)
        lines = [ (' ' * len(parents)) + l for l in lines ]

    blame = []
    for parent in parents:
        blame_commit = map[parent.id]
        blame_blob = get_tree_data(blame_repo, blame_commit.tree, path)
        if blame_blob:
            blame_lines = splitlines(blame_blob.data)
            blame.append(blame_lines)
        else:
            blame.append(None)
        
    output = []
    new_lineno = 1
    old_lineno = [1]*len(parents)
    for line in lines:
        origin = line[0:len(parents)]
        content = line[len(parents):]

        cur_blame = None
        for i in range(len(parents)):
            if '-' in origin:
                if origin[i] == '-':
                    cur_blame = blame[i][old_lineno[i] - 1]
                    old_lineno[i] += 1
            else:
                if origin[i] != '+':
                    cur_blame = blame[i][old_lineno[i] - 1]
                    old_lineno[i] += 1

        if '-' not in origin:
            lno = new_lineno
            new_lineno += 1
        else:
            lno = None

        output.append((lno, cur_blame, origin, content))

    print >>f, '<table>'
    print >>f, '<tr>'

    def color(origin):
        if origin == '+':
            return 'blue'
        elif origin == '-':
            return 'red'
        else:
            return 'black'

    print >>f, '<td><pre>'
    for (lno, blame, origin, line) in output:
        if lno:
            print >>f, ('<code id="%d">' % lno), lno, '</code>'
        else:
            print >>f, ''
    print >>f, '</pre></td>'

    print >>f, '<td><pre>'
    for (lno, blame, origin, line) in output:
        if blame:
            (rev, fname, line, author) = blame.split(':', 3)
            if fname == '%':
                fname = path
            print >>f, ('<a href="/commit/%s/%s#%s">' % (rev, fname, line)) + rev[:6] + '/' + author[:20] + '</a>'
        else:
            print >>f, ''
    print >>f, '</pre></td>'
    
    print >>f, '<td><pre>'
    for (lno, blame, origin, line) in output:
        print >>f, '<code style="color: %s;">%s</code>' % (color(origin), origin)
    print >>f, '</pre></td>'

    print >>f, '<td><pre>'
    for (lno, blame, origin, line) in output:
        print >>f, '<code style="color: %s;">%s</code>' % (color(origin), cgi.escape(line))
    print >>f, '</pre></td>'

    print >>f, '</table>'

def show_commit(f, commit, path):
    parents = commit.parents

    msg = commit.message
    msg_lines = splitlines(msg)
    header = linkify(cgi.escape(msg_lines[0]))

    def fmt_rev(rev):
        return '<a href="/commit/%s">%s</a>' % (rev, rev)

    print >>f, '<html>'
    print >>f, '<head>'
    if path:
        print >>f, '<title>Blame - %s (%s)</title>' % (path, commit.id)
    else:
        print >>f, '<title>Commit %s</title>' % commit.id
    print >>f, '</head>'

    print >>f, '<body>'
    print >>f, '<h3>' + header + '</h3>'
    print >>f, '<pre><code>' + cgi.escape('\n'.join(msg_lines[1:])) + '</code></pre>'

    print >>f, '<table>'
    print >>f, '<tr><td>commit</td><td>' + fmt_rev(commit.id) + '</td></tr>'
    for parent in parents:
        print >>f, '<tr>'
        print >>f, '<td>parent</td>'
        print >>f, '<td>' + fmt_rev(parent.id) + '</td>'
        print >>f, '</tr>'

    def fmt_sig(signature):
        return cgi.escape(signature.name) + ' &lt;' + cgi.escape(signature.email) + '&gt;'

    print >>f, '<tr><td>author</td><td>' + fmt_sig(commit.author) + '</td></tr>'
    print >>f, '<tr><td>committer</td><td>' + fmt_sig(commit.committer) + '</td></tr>'

    t = datetime.fromtimestamp(commit.commit_time,
                               MyTimezone(commit.commit_time_offset))
    print >>f, '<tr><td>commit time</td><td>' + t.ctime() + '</td></tr>'

    print >>f, '</table>'

    difftxt = runCmd(['/usr/bin/git', 'show', '--cc', '--pretty=format:', '--raw', str(commit.id)],
                     stdout=subprocess.PIPE, cwd=tree_root)
    if not difftxt:
        return

    difflines = splitlines(difftxt)
    file_changes = []
    for line in difflines:
        # Skip colons.
        line = line[len(parents):]

        prefix = 2 * (len(parents) + 1)
        data = line.split(' ', prefix)
        data = data[prefix]
        data = data.split('\t')

        file_changes.append(data)

    print >>f, '<ul>'

    for change in file_changes:
        print >>f, '<li>%s <a href="/commit/%s/%s">%s</a>' % (change[0], commit.id, change[1], cgi.escape(change[1]))
    
    print >>f, '</ul>'

    if path:
        show_file(f, commit, path)

    print >>f, '</body>'
    print >>f, '</html>'

map = {}
for commit in blame_repo.walk(blame_repo.head.target):
    orig = pygit2.Oid(hex=commit.message.split()[1])
    map[orig] = commit
    
class Handler(SimpleHTTPServer.SimpleHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200, 'OK')
        self.send_header('Content-type', 'text/html')
        self.end_headers()

        parts = self.path.split('/')
        if parts[0] == '' and parts[1] == 'commit':
            commit = repo.get(parts[2])
            path = None
            if len(parts) > 3:
                path = '/'.join(parts[3:])

        show_commit(self.wfile, commit, path)

print 'Serving...'
        
server_address = ('', 8000)
httpd = SocketServer.TCPServer(server_address, Handler)
httpd.serve_forever()
