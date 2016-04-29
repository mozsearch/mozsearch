import SimpleHTTPServer
from BaseHTTPServer import HTTPServer
from SocketServer import ForkingMixIn
import urllib
import urlparse
import sys
import os
import os.path
import json
import re
import subprocess
import signal
import time
import errno
import traceback

import crossrefs
import codesearch
import blame
from logger import log

def parse_search(searchString):
    pieces = searchString.split(' ')
    result = {}
    for i in range(len(pieces)):
        if pieces[i].startswith('path:'):
            result['pathre'] = re.escape(pieces[i][len('path:'):])
        elif pieces[i].startswith('pathre:'):
            result['pathre'] = pieces[i][len('pathre:'):]
        elif pieces[i].startswith('symbol:'):
            result['symbol'] = pieces[i][len('symbol:'):].strip().replace('.', '#')
        elif pieces[i].startswith('re:'):
            result['re'] = (' '.join(pieces[i:]))[len('re:'):]
            break
        else:
            result['default'] = re.escape(' '.join(pieces[i:]))
            break

    return result

def is_trivial_search(parsed):
    if 'symbol' in parsed:
        return False

    for k in parsed:
        if len(parsed[k]) >= 3:
            return False

    return True

def sort_results(results):
    def is_test(p):
        return '/test/' in p or '/tests/' in p or '/mochitest/' in p or '/unit/' in p or 'testing/' in p

    def prio(p):
        if is_test(p): return 0
        elif '__GENERATED__' in p: return 1
        else: return 2

    # neg if p1 is before p2
    def sortfunc(p1, p2):
        prio1 = prio(p1)
        prio2 = prio(p2)
        r = cmp(p1, p2)
        if prio1 < prio2:
            r += 10000
        elif prio1 > prio2:
            r -= 10000
        return r

    def combine_lines(lines1, lines2):
        # Eliminate duplicates and sort by line number.
        dict1 = { l['lno']: l for l in lines1 }
        dict2 = { l['lno']: l for l in lines2 }
        dict1.update(dict2)
        lines = dict1.values()
        lines.sort(lambda l1, l2: cmp(l1['lno'], l2['lno']))
        return lines

    def combine(path1r, path2r):
        return {'path': path1r['path'],
                'lines': combine_lines(path1r['lines'], path2r['lines'])}

    def sort_inner(results):
        m = {}
        for result in results:
            m[result['path']] = combine(m.get(result['path'], result), result)

        paths = m.keys()
        paths.sort(sortfunc)

        return [ m[path] for path in paths ]

    return { kind: sort_inner(res) for kind, res in results.items() }

def search_files(path):
    pathFile = os.path.join(indexPath, 'repo-files')
    try:
        results = subprocess.check_output(['grep', '-Ei', path, pathFile])
    except:
        return []
    results = results.strip().split('\n')
    results = [ {'path': f, 'lines': []} for f in results ]
    return results[:1000]

def get_json_search_results(query):
    try:
        searchString = query['q'][0]
    except:
        searchString = ''

    try:
        foldCase = query['case'][0] != 'true'
    except:
        foldCase = True

    parsed = parse_search(searchString)
    if is_trivial_search(parsed):
        results = {}
        results['query'] = searchString
        return json.dumps(results)

    if 'symbol' in parsed:
        # FIXME: Need to deal with path here
        symbols = parsed['symbol']
        results = crossrefs.lookup(symbols)
    elif 're' in parsed:
        path = parsed.get('pathre', '.*')
        substrResults = codesearch.search(parsed['re'], foldCase, file = path)
        results = {'default': substrResults}
    elif 'default' in parsed:
        path = parsed.get('pathre', '.*')
        substrResults = codesearch.search(parsed['default'], foldCase, file = path)
        if 'pathre' in parsed:
            fileResults = []
        else:
            fileResults = search_files(parsed['default'])
        results = {'default': fileResults + substrResults}
    elif 'pathre' in parsed:
        path = parsed['pathre']
        results = {"default": search_files(path)}
    else:
        results = {}

    results = sort_results(results)
    results['query'] = searchString
    return json.dumps(results)

class Handler(SimpleHTTPServer.SimpleHTTPRequestHandler):
    def do_GET(self):
        pid = os.fork()
        if pid:
            # Parent process
            log('request(handled by %d) %s', pid, self.path)

            timedOut = [False]
            def handler(signum, frame):
                log('timeout %d, killing', pid)
                timedOut[0] = True
                os.kill(pid, signal.SIGKILL)
            signal.signal(signal.SIGALRM, handler)
            signal.alarm(15)

            t = time.time()
            while True:
                try:
                    (pid2, status) = os.waitpid(pid, 0)
                    break
                except OSError, e:
                    if e.errno != errno.EINTR: raise e

            failed = timedOut[0]
            if os.WIFEXITED(status) and os.WEXITSTATUS(status) != 0:
                log('error pid %d - %f', pid, time.time() - t)
                failed = True
            else:
                log('finish pid %d - %f', pid, time.time() - t)

            if failed:
                self.send_response(504)
                self.end_headers()
        else:
            # Child process
            try:
                self.process_request()
                os._exit(0)
            except:
                e = traceback.format_exc()
                log('exception\n%s', e)
                os._exit(1)

    def log_request(self, *args):
        pass

    def process_request(self):
        url = urlparse.urlparse(self.path)
        pathElts = url.path.split('/')

        # Strip any extra slashes.
        pathElts = [ elt for elt in pathElts if elt != '' ]

        if pathElts == []:
            filename = os.path.join(indexPath, 'help.html')
            data = open(filename).read()
            self.generate(data, 'text/html')
        elif pathElts[:2] == ['mozilla-central', 'source']:
            filename = os.path.join(indexPath, 'file', '/'.join(pathElts[2:]))
            try:
                data = open(filename).read()
            except:
                filename = os.path.join(indexPath, 'dir', '/'.join(pathElts[2:]), 'index.html')
                try:
                    data = open(filename).read()
                except:
                    return SimpleHTTPServer.SimpleHTTPRequestHandler.do_GET(self)

            self.generate(data, 'text/html')
        elif pathElts[:2] == ['mozilla-central', 'search']:
            query = urlparse.parse_qs(url.query)
            j = get_json_search_results(query)
            if 'json' in self.headers.getheader('Accept', ''):
                self.generate(j, 'application/json')
            else:
                try:
                    title = query['q'][0]
                except:
                    title = ''
                j = j.replace("/script", "\\/script")
                template = os.path.join(indexPath, 'templates/search.html')
                self.generateWithTemplate({'{{BODY}}': j, '{{TITLE}}': title}, template)
        elif pathElts[:2] == ['mozilla-central', 'define']:
            query = urlparse.parse_qs(url.query)
            symbol = query['q'][0]
            results = crossrefs.lookup(symbol)
            definition = results['Definitions'][0]
            filename = definition['path']
            lineno = definition['lines'][0]['lno']
            url = '/mozilla-central/source/' + filename + '#' + str(lineno)

            self.send_response(301)
            self.send_header("Location", url)
            self.end_headers()
        elif pathElts[:2] == ['mozilla-central', 'commit']:
            rev = pathElts[2]
            filename = '/'.join(pathElts[3:])

            self.send_response(200)
            self.send_header('Content-type', 'text/html')
            self.end_headers()

            blame.show_commit(self.wfile, rev, filename)
        elif pathElts[:2] == ['mozilla-central', 'rev']:
            rev = pathElts[2]
            filename = '/'.join(pathElts[3:])

            self.send_response(200)
            self.send_header('Content-type', 'text/html')
            self.end_headers()

            blame.show_rev(self.wfile, rev, filename)
        elif pathElts[:2] == ['mozilla-central', 'commit-info']:
            rev = pathElts[2]

            self.send_response(200)
            self.send_header('Content-type', 'application/json')
            self.end_headers()

            blame.get_commit_info(self.wfile, rev)
        else:
            return SimpleHTTPServer.SimpleHTTPRequestHandler.do_GET(self)

    def generate(self, data, type):
        self.send_response(200)
        self.send_header("Content-type", type)
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()

        self.wfile.write(data)

    def generateWithTemplate(self, replacements, templateFile):
        output = open(templateFile).read()
        for (k, v) in replacements.items():
            output = output.replace(k, v)

        self.send_response(200)
        self.send_header("Content-type", "text/html")
        self.send_header("Content-Length", str(len(output)))
        self.end_headers()

        self.wfile.write(output)

if len(sys.argv) > 1:
    config_fname = sys.argv[1]
else:
    config_fname = 'router-config.json'

config = json.load(open(config_fname))

mozSearchPath = config['moz-search-path']
indexPath = config['index-path']

crossrefs.load(config)
codesearch.load(config)
blame.load(config)

class ForkingServer(ForkingMixIn, HTTPServer):
    pass

server_address = ('', 8000)
httpd = ForkingServer(server_address, Handler)
httpd.serve_forever()
