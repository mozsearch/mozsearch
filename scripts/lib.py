from __future__ import absolute_import
from __future__ import print_function
import sys
import subprocess

def run(cmd, **extra):
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **extra)
    (stdout, stderr) = p.communicate()

    if p.returncode:
        print('Command failed', cmd, file=sys.stderr)
        print('Return code', p.returncode, file=sys.stderr)
        print(stdout, file=sys.stderr)
        print('---', file=sys.stderr)
        print(stderr, file=sys.stderr)
        sys.exit(p.returncode)

    return stdout
