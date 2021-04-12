from __future__ import absolute_import
import sys
import socket
import os
import os.path
import time
from logger import log

import grpc
from src.proto import livegrep_pb2
from src.proto import livegrep_pb2_grpc

def collateMatches(matches):
    paths = {}
    for m in matches:
        # For results in the "mozilla-subrepo" repo, which is the mozilla/
        # subfolder of comm-central, we need to adjust the path to reflect
        # the fact that it's in the subfolder.
        path = m.path
        if m.tree == 'mozilla-subrepo':
            path = 'mozilla/' + path

        line = {
            'lno': m.line_number,
            'bounds': [m.bounds.left, m.bounds.right],
            'line': m.line
        }

        if len(m.context_before):
            line['context_before'] = list(m.context_before)
        if len(m.context_after):
            line['context_after'] = list(m.context_after)

        paths.setdefault(path, []).append(line)
    results = [ {'path': p, 'icon': '', 'lines': paths[p]} for p in paths ]
    return results

def do_search(host, port, pattern, fold_case, file, context_lines):
    query = livegrep_pb2.Query(line = pattern, file = file, fold_case = fold_case,
                               context_lines = context_lines)
    log('QUERY %s', repr(query).replace('\n', ', '))

    channel = grpc.insecure_channel('{0}:{1}'.format(host, port))
    grpc_stub = livegrep_pb2_grpc.CodeSearchStub(channel)
    result = grpc_stub.Search(query) # maybe add a timeout arg here?
    channel.close()

    matches = collateMatches(result.results)
    log('codesearch result with %d matches', len(matches))
    return (matches, livegrep_pb2.SearchStats.ExitReason.Name(result.stats.exit_reason) == 'TIMEOUT')

def daemonize(args):
    # Spawn a process to start the daemon
    pid = os.fork()
    if pid:
        # Parent
        return

    # Double fork
    pid = os.fork()
    if pid:
        os._exit(0)

    pid = os.fork()
    if pid:
        os._exit(0)

    si = open('/dev/null', 'r')
    so = open('/dev/null', 'a+')
    se = open('/dev/null', 'a+')
    os.dup2(si.fileno(), sys.stdin.fileno())
    os.dup2(so.fileno(), sys.stdout.fileno())
    os.dup2(se.fileno(), sys.stderr.fileno())

    os.execvp(args[0], args)

def startup_codesearch(data):
    log('Starting codesearch')

    args = ['codesearch', '-grpc', 'localhost:' + str(data['codesearch_port']),
            '--noreuseport',
            '-load_index', data['codesearch_path'],
            '-max_matches', '1000', '-timeout', '10000']

    daemonize(args)
    time.sleep(5)

def search(pattern, fold_case, path, tree_name, context_lines):
    data = tree_data[tree_name]

    try:
        return do_search('localhost', data['codesearch_port'], pattern, fold_case, path, context_lines)
    except Exception as e:
        log('Got exception: %s', repr(e))
        if e.code() != grpc.StatusCode.UNAVAILABLE:
            # TODO: better job of surfacing the error back to the user. This might be e.g.
            # a grpc.StatusCode.INVALID_ARGUMENT if say the `pattern` is a malformed regex
            return ([], False)

        # If the exception indicated a connection failure, try to restart the server and search
        # again.
        startup_codesearch(data)
        try:
            return do_search('localhost', data['codesearch_port'], pattern, fold_case, path, context_lines)
        except Exception as e:
            log('Got exception after restarting codesearch: %s', repr(e))
            # TODO: as above, do a better job of surfacing the error back to the user.
            return ([], False)


def load(config):
    global tree_data
    tree_data = {}
    for tree_name in config['trees']:
        tree_data[tree_name] = {
            'codesearch_path': config['trees'][tree_name]['codesearch_path'],
            'codesearch_port': config['trees'][tree_name]['codesearch_port'],
        }
        # Start the daemon during loading. If it dies we will restart it lazily
        # during the search function, but that should be rare. This avoids a
        # race condition where search() can get invoked multiple times in quick
        # succession by separate queries, resulting in the daemon getting started
        # multiple times.
        startup_codesearch(tree_data[tree_name])
