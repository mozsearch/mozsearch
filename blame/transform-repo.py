#!/usr/bin/env python

import os
import os.path
import sys
import subprocess
import re
import time

oldTree = sys.argv[1]
newTree = sys.argv[2]

map = {}
blameCache = {}

cmdTime = 0

def runCmd(*args, **kwargs):
    global cmdTime

    t0 = time.time()
    p = subprocess.Popen(*args, **kwargs)
    (stdout, stderr) = p.communicate()
    t1 = time.time()

    cmdTime += t1-t0
    print '   ', (t1-t0), ' '.join(args[0])

    return stdout

def getBlameFor(rev, filename):
    if (rev, filename) in blameCache:
        return blameCache[(rev, filename)]

    result = runCmd(['/usr/bin/git', 'show', map[rev] + ':' + filename],
                    stdout=subprocess.PIPE, cwd=newTree)
    result = result.splitlines()
    #blameCache[(rev, filename)] = result
    return result

def modifyBlameForMove(blame, blameFilename):
    for i in range(len(blame)):
        print '  ==>', blame[i]
        data = blame[i].split(' ')
        if len(data) == 3:
            continue
        blame[i] = ' '.join([data[0], data[1], blameFilename])

lineId = 1

def transformViaBlame(parents, fileStatus, rev):
    global cmdTime, lineId

    t00 = time.time()

    for (status, filename) in fileStatus:
        print ' ', status, filename

        if status == 'D':
            continue

        parentOps = [ '^' + p for p in parents ]
        blameData = runCmd(['/usr/bin/git', 'blame', '--incremental'] +
                           parentOps + [rev, '--', filename],
                           stdout=subprocess.PIPE, cwd=oldTree)
        blameData = blameData.replace('\r', '').splitlines()
        blameData = [ l.strip() for l in blameData ]

        newBlame = []

        def fill(newBlame, src, start):
            if start + len(src) > len(newBlame):
                newBlame += [''] * (len(src) + start - len(newBlame))
            newBlame[start : start+len(src)] = src

        start = True
        for line in blameData:
            if start:
                start = False
                (sha, sourceLine, resultLine, numLines) = line.strip().split(' ')
                sourceLine = int(sourceLine)
                resultLine = int(resultLine)
                numLines = int(numLines)

                sourceLine -= 1
                resultLine -= 1
            else:
                if line.startswith('filename '):
                    start = True
                    (tag, blameFilename) = line.strip().split(' ', 1)

                    if sha == rev:
                        lines = [ sha + ' ' + str(id + lineId) for id in range(numLines) ]
                        lineId += numLines
                        fill(newBlame, lines, resultLine)
                    else:
                        src = getBlameFor(sha, blameFilename)
                        if blameFilename != filename:
                            modifyBlameForMove(src, blameFilename)
                        fill(newBlame, src[sourceLine : sourceLine+numLines], resultLine)

        #blameCache[(rev, filename)] = newBlame

        if status == 'A':
            runCmd(['/bin/mkdir', '-p', os.path.join(newTree, os.path.dirname(filename))])

        f = open(os.path.join(newTree, filename), 'w')
        f.write(''.join([ b + '\n' for b in newBlame ]))
        f.close()

    filenames = [ f[1] for f in fileStatus if f[0] != 'D' ]
    if len(filenames):
        runCmd(['/usr/bin/git', 'add'] + filenames, cwd=newTree)

    deletions = [ f[1] for f in fileStatus if f[0] == 'D' ]
    if len(deletions):
        runCmd(['/usr/bin/git', 'rm', '--cached'] + deletions, cwd=newTree)

    treeId = runCmd(['/usr/bin/git', 'write-tree'],
                    stdout=subprocess.PIPE, cwd=newTree)
    treeId = treeId.strip()

    map[rev] = treeId2

    parentArgs = []
    for p in parents:
        parentArgs += ['-p', map[p]]

    commitId = runCmd(['/usr/bin/git', 'commit-tree', treeId] + parentArgs + ['-m', 'Rev ' + rev],
                      stdout=subprocess.PIPE, cwd=newTree)
    commitId = commitId.strip()

    print '  =>', commitId

    map[rev] = commitId

    runCmd(['/usr/bin/git', 'update-ref', 'refs/heads/master', commitId],
           stdout=subprocess.PIPE, cwd=newTree)

    print ' ', cmdTime, 'CMD'
    cmdTime = 0

    t11 = time.time()
    print ' ', (t11-t00), 'TOTAL'

    return commitId

def transformRevision(rev):
    stdout = runCmd(['/usr/bin/git', 'show', '--pretty=format:%P', '--name-status', rev],
                    stdout=subprocess.PIPE, cwd=oldTree)
    
    lines = stdout.splitlines()

    parents = lines[0].split()
    fileStatus = [ l.split('\t', 1) for l in lines[1:] ]

    return transformViaBlame(parents, fileStatus, rev)

print 'NOW =', time.asctime()

finished = runCmd(['/usr/bin/git', 'log', '--oneline', '--no-abbrev-commit', '--topo-order'],
                stdout=subprocess.PIPE, cwd=newTree)
finished = finished.replace('\r', '').splitlines()
map = { l.split()[2]: l.split()[0] for l in finished }

commits = runCmd(['/usr/bin/git', 'log', '--pretty=oneline', '--topo-order', '--reverse'],
                stdout=subprocess.PIPE, cwd=oldTree)
commits = commits.replace('\r', '').splitlines()
commits = [ l.split()[0] for l in commits ]

for rev in commits:
    if rev in map:
        continue

    print 'Transforming', rev, time.asctime()

    newRev = transformRevision(rev)
