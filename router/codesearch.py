#!/usr/bin/env python3

from __future__ import absolute_import
import json
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
            # The before context is provided in reverse order which is not what
            # we want.
            before = list(m.context_before)
            # This does not return the list, so it's on its own line.
            before.reverse()
            line['context_before'] = before
        if len(m.context_after):
            line['context_after'] = list(m.context_after)

        paths.setdefault(path, []).append(line)
    results = [ {'path': p, 'icon': '', 'lines': paths[p]} for p in paths ]
    return results

def do_search(host, port, pattern, fold_case, file, context_lines):
    t = time.time()
    query = livegrep_pb2.Query(line = pattern, file = file, fold_case = fold_case,
                               context_lines = context_lines)
    log('QUERY %s', repr(query).replace('\n', ', '))

    channel = grpc.insecure_channel('{0}:{1}'.format(host, port))
    grpc_stub = livegrep_pb2_grpc.CodeSearchStub(channel)
    result = grpc_stub.Search(query) # maybe add a timeout arg here?
    channel.close()

    matches = collateMatches(result.results)
    log('  codesearch result with %d line matches across %d paths - %f : %s',
        len(result.results), len(matches), time.time() - t,
        repr(result.stats).replace('\n', ', '))
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

def stop_codesearch(data):
    log('Stopping codesearch on port %d', data['codesearch_port'])
    os.system("pkill -f '^codesearch.+localhost:%d '" % (data['codesearch_port']))

def startup_codesearch(data):
    log('Starting codesearch on port %d', data['codesearch_port'])

    use_threads = 4
    try:
        # Defined to return None if "undetermined", so we handle that but also
        # are prepared for things to throw.
        maybe_count = os.cpu_count()
        # Limit us to 8 cores primarily to avoid the Vagrant VM getting too
        # resource hungry.  We may tend to want to give it as many cores as
        # possible for rust compilation, but our current EC2 core max is 8.
        if maybe_count is not None:
            use_threads = min(8, maybe_count)
    except:
        pass

    args = ['codesearch', '-grpc', 'localhost:' + str(data['codesearch_port']),
            '--noreuseport',
            '-load_index', data['codesearch_path'],
            # Note that because multiple threads are involved, this limit
            # potentially will not return the same results every time it is run
            # and that's okay.  But because of our app-level caching, it ends
            # up that we will usually only run one exact query once.
            '-max_matches', '1000',
            '-threads', f'{use_threads}',
            # We set the timeout to 30 seconds up from 10 seconds because our
            # caching policy requires our searches to be deterministic in the
            # face of I/O slowness.  Note that this differs from the
            # non-determinism of the "max_matches" limit which is acceptable.
            # We do expect to have addressed this problem by ensuring the cache
            # is fully loaded via vmtouch before serving begins in earnest.
            '-timeout', '30000',
            '-context_lines', '0']

    daemonize(args)
    # Sleep a teeny bit to let the server have some exclusive time to spin up
    # before any siblings start to race it.
    time.sleep(0.1)

def try_info_request(host, port):
    infoq = livegrep_pb2.InfoRequest()

    channel = grpc.insecure_channel('{0}:{1}'.format(host, port))
    grpc_stub = livegrep_pb2_grpc.CodeSearchStub(channel)
    result = grpc_stub.Info(infoq) # maybe add a timeout arg here?
    channel.close()

def wait_for_codesearch(data, max_tries=200):
    '''Wait for the codesearch server to become available/responsive.'''

    tries = 0
    while tries < max_tries:
        tries += 1
        try:
            try_info_request('localhost', data['codesearch_port'])
            break
        except Exception as e:
            # sleep a little to give the server time to make progress
            time.sleep(0.1)
    log('Server on port %d found alive after %d tries', data['codesearch_port'], tries)

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
        stop_codesearch(data)
        startup_codesearch(data)
        try:
            return do_search('localhost', data['codesearch_port'], pattern, fold_case, path, context_lines)
        except Exception as e:
            log('Got exception after restarting codesearch: %s', repr(e))
            # TODO: as above, do a better job of surfacing the error back to the user.
            return ([], False)


def load(config, stop=True, start=True, only_tree_name=None):
    global tree_data
    tree_data = {}
    for tree_name in config['trees']:
        if only_tree_name and tree_name != only_tree_name:
            continue
        tree_data[tree_name] = {
            'codesearch_path': config['trees'][tree_name]['codesearch_path'],
            'codesearch_port': config['trees'][tree_name]['codesearch_port'],
        }
        # Start the daemon during loading. If it dies we will restart it lazily
        # during the search function, but that should be rare. This avoids a
        # race condition where search() can get invoked multiple times in quick
        # succession by separate queries, resulting in the daemon getting started
        # multiple times.
        if stop:
            stop_codesearch(tree_data[tree_name])
        if start:
            startup_codesearch(tree_data[tree_name])
            wait_for_codesearch(tree_data[tree_name])

if __name__ == '__main__':
    '''(Re)start or stop all the codesearch instances for the given config file.

    Usage:
    codesearch.py CONFIG.JSON start [only_tree_name]
    codesearch.py CONFIG.JSON stop [only_tree_name]
    '''
    stop = True
    start = True
    only_tree_name = None
    if sys.argv[2] == 'stop':
        start = False
    if len(sys.argv) > 3:
        only_tree_name = sys.argv[3]

    config = json.load(open(sys.argv[1]))
    load(config, stop=stop, start=start, only_tree_name=only_tree_name)
