import sys
import subprocess

def run(cmd, **extra):
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, **extra)
    (stdout, stderr) = p.communicate()

    if p.returncode:
        print >>sys.stderr, 'Command failed', cmd
        print >>sys.stderr, stdout
        print >>sys.stderr, '---'
        print >>sys.stderr, stderr
        sys.exit(p.returncode)

    return stdout
