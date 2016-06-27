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
from logger import log

# TODO:
# Move spinner to the right end?
# Make a help box?

def index_path(tree_name):
    return config['repos'][tree_name]['index_path']

# Simple globbing implementation, except ^ and $ are also allowed.
def parse_path_filter(filter):
    filter = filter.replace('(', '\\(')
    filter = filter.replace(')', '\\)')
    filter = filter.replace('|', '\\|')
    filter = filter.replace('.', '\\.')

    def star_repl(m):
        if m.group(0) == '*':
            return '[^/]*'
        else:
            return '.*'
    filter = re.sub(r'\*\*|\*', star_repl, filter)

    filter = filter.replace('?', '.')

    def repl(m):
        s = m.group(1)
        components = s.split(',')
        s = '|'.join(components)
        return '(' + s + ')'
    filter = re.sub('{([^}]*)}', repl, filter)

    return filter

def parse_search(searchString):
    pieces = searchString.split(' ')
    result = {}
    for i in range(len(pieces)):
        if pieces[i].startswith('path:'):
            result['pathre'] = parse_path_filter(pieces[i][len('path:'):])
        elif pieces[i].startswith('pathre:'):
            result['pathre'] = pieces[i][len('pathre:'):]
        elif pieces[i].startswith('symbol:'):
            result['symbol'] = ' '.join(pieces[i:])[len('symbol:'):].strip().replace('.', '#')
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

def search_files(tree_name, path):
    pathFile = os.path.join(index_path(tree_name), 'repo-files')
    try:
        # We set the locale to make grep much faster.
        results = subprocess.check_output(['grep', '-Ei', path, pathFile], env={'LC_CTYPE': 'C'})
    except:
        return []
    results = results.strip().split('\n')
    results = [ {'path': f, 'lines': []} for f in results ]
    return results[:1000]

def get_json_search_results(tree_name, query):
    try:
        searchString = query['q'][0]
    except:
        searchString = ''

    try:
        foldCase = query['case'][0] != 'true'
    except:
        foldCase = True

    try:
        regexp = query['regexp'][0] == 'true'
    except:
        regexp = False

    try:
        pathFilter = query['path'][0]
    except:
        pathFilter = ''

    parsed = parse_search(searchString)

    if pathFilter:
        parsed['pathre'] = parse_path_filter(pathFilter)

    if regexp:
        if 'default' in parsed:
            del parsed['default']
        if 're' in parsed:
            del parsed['re']
        parsed['re'] = searchString

    if 'default' in parsed and len(parsed['default']) == 0:
        del parsed['default']

    if is_trivial_search(parsed):
        results = {}
        return json.dumps(results)

    title = searchString
    if not title:
        title = 'Files ' + pathFilter

    if 'symbol' in parsed:
        # FIXME: Need to deal with path here
        symbols = parsed['symbol']
        title = 'Symbol ' + symbols
        results = crossrefs.lookup(tree_name, symbols)
    elif 're' in parsed:
        path = parsed.get('pathre', '.*')
        substrResults = codesearch.search(parsed['re'], foldCase, path, tree_name)
        results = {'default': substrResults}
    elif 'default' in parsed:
        path = parsed.get('pathre', '.*')
        substrResults = codesearch.search(parsed['default'], foldCase, path, tree_name)
        if 'pathre' in parsed:
            fileResults = []
        else:
            fileResults = search_files(tree_name, parsed['default'])
        results = {'default': fileResults + substrResults}
    elif 'pathre' in parsed:
        path = parsed['pathre']
        results = {"default": search_files(tree_name, path)}
    else:
        assert False
        results = {}

    results = sort_results(results)
    results['*title*'] = title
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
        path_elts = url.path.split('/')

        # Strip any extra slashes.
        path_elts = [ elt for elt in path_elts if elt != '' ]

        if not path_elts:
            filename = os.path.join(index_path('nss'), 'help.html')
            data = open(filename).read()
            self.generate(data, 'text/html')
        elif len(path_elts) >= 2 and path_elts[1] == 'source':
            tree_name = path_elts[0]
            filename = os.path.join(index_path(tree_name), 'file', '/'.join(path_elts[2:]))
            try:
                data = open(filename).read()
            except:
                filename = os.path.join(index_path(tree_name), 'dir', '/'.join(path_elts[2:]), 'index.html')
                try:
                    data = open(filename).read()
                except:
                    return SimpleHTTPServer.SimpleHTTPRequestHandler.do_GET(self)

            self.generate(data, 'text/html')
        elif path_elts[1] == 'search':
            tree_name = path_elts[0]
            query = urlparse.parse_qs(url.query)
            j = get_json_search_results(tree_name, query)
            if 'json' in self.headers.getheader('Accept', ''):
                self.generate(j, 'application/json')
            else:
                j = j.replace("/script", "\\/script")
                template = os.path.join(index_path(tree_name), 'templates/search.html')
                self.generateWithTemplate({'{{BODY}}': j, '{{TITLE}}': 'Search'}, template)
        elif path_elts[1] == 'define':
            tree_name = path_elts[0]
            query = urlparse.parse_qs(url.query)
            symbol = query['q'][0]
            results = crossrefs.lookup(tree_name, symbol)
            definition = results['Definitions'][0]
            filename = definition['path']
            lineno = definition['lines'][0]['lno']
            url = '/' + tree_name + '/source/' + filename + '#' + str(lineno)

            self.send_response(301)
            self.send_header("Location", url)
            self.end_headers()
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
    config_fname = 'config.json'

config = json.load(open(config_fname))

crossrefs.load(config)
codesearch.load(config)

class ForkingServer(ForkingMixIn, HTTPServer):
    pass

server_address = ('', 8000)
httpd = ForkingServer(server_address, Handler)
httpd.serve_forever()
