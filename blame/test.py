import sys
import pygit2
import os.path
import unicodedata
from datetime import datetime, tzinfo, timedelta
import time
import email
import email.utils
import subprocess

path = sys.argv[1]
hg_path = sys.argv[2]

repo = pygit2.Repository(pygit2.discover_repository(path))

class MyTimezone(tzinfo):
    def __init__(self, offset):
        self.offset = offset

    def utcoffset(self, dt):
        return timedelta(minutes=self.offset)

    def dst(self, dt):
        return timedelta(0)

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

def find_hg_commit(commit):
    if not hg_path:
        return None

    if commit.author.email == 'none@none':
        commit_user = commit.author.name
    else:
        commit_user = '%s <%s>' % (commit.author.name, commit.author.email)

    t = datetime.fromtimestamp(commit.commit_time,
                               MyTimezone(commit.commit_time_offset))
    timestamp = time.mktime(t.timetuple())
    commit_date = email.utils.formatdate(timestamp)

    search_str = splitlines(commit.message)[0]

    print '*', commit_user, commit_date, search_str

    out = run_cmd(['hg', 'log', '-R', hg_path,
                   '--user', commit_user,
                   '--date', commit_date,
                   '--template', '{node} {desc|firstline}\n'],
                  stdout=subprocess.PIPE)
    lines = splitlines(out)

    if len(lines) == 1:
        return lines[0].split(' ')[0]
    else:
        return None

for commit in repo.walk(repo.head.target):
    print str(commit.id)
    print find_hg_commit(commit)
