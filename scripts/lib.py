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
        print(stdout.decode(), file=sys.stderr)
        print('---', file=sys.stderr)
        print(stderr.decode(), file=sys.stderr)
        sys.exit(p.returncode)

    return stdout

def run_showing_output(cmd, output_filter=None, **extra):
    print('running', repr(cmd))
    p = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **extra)
    (stdout, stderr) = p.communicate()

    if p.returncode:
        print('Command failed', cmd, file=sys.stderr)
        print('Return code', p.returncode, file=sys.stderr)
        print(stdout, file=sys.stderr)
        print('---', file=sys.stderr)
        print(stderr, file=sys.stderr)
        sys.exit(p.returncode)
    else:
        stdout = stdout.decode()
        stderr = stderr.decode()

        if output_filter is not None:
            stdout = output_filter(stdout)
            stderr = output_filter(stderr)

        print('--- stdout')
        print(stdout)
        print('--- stderr')
        print(stderr)
        print('--- (end output)')

    return stdout
