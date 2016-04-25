#!/usr/bin/env python

import os
import os.path
import sys
import subprocess
import re

treeRoot = sys.argv[1]
filePath = sys.argv[2]

def applyDiff(curBlame, diff, rev):
    curBlame = curBlame[:]

    lines = diff.split('\n')

    while True:
        if not lines[0]:
            # This could be a merge commit that doesn't actually have any changes.
            return curBlame

        if lines[0][0] == '@':
            break
        else:
            lines = lines[1:]

    curLine = -1
    for line in lines:
        if not line:
            continue

        #print curLine
        #print line

        if line[0] == '@':
            m = re.match(r'@@ -([0-9]+),[0-9]+ \+([0-9]+),[0-9]+ @@', line)
            curLine = int(m.group(2)) - 1
        elif line[0] == ' ':
            curLine += 1
        elif line[0] == '-':
            curBlame.pop(curLine)
        elif line[0] == '+':
            curBlame[curLine:curLine] = [rev]
            curLine += 1
        else:
            raise "Unknown diff line: '{}'".format(line)

        #print curBlame

    #print 'done'

    return curBlame

p = subprocess.Popen(['/usr/bin/git', 'rev-list', 'HEAD', '--', filePath],
                     stdout=subprocess.PIPE, cwd=treeRoot)
(stdout, stderr) = p.communicate()

revs = stdout.split()
revs = reversed(revs)

curBlame = []

for rev in revs:
    p = subprocess.Popen(['/usr/bin/git', 'show', '--oneline', rev, filePath],
                         stdout=subprocess.PIPE, cwd=treeRoot)
    (stdout, stderr) = p.communicate()

    curBlame = applyDiff(curBlame, stdout, rev)

    print '--', rev, '--'
    #print '\n'.join(curBlame)

#print '\n'.join(curBlame)
