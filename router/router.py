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
import collections

import crossrefs
import identifiers
import codesearch
from logger import log

#FIXME:
# If you have an identifier search that includes lots of symbols, it will be very slow. Need to limit the result count, but we need to return 1000 results even if there are dupes.
# Need case insensitivity.
# Path restriction?

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
        elif pieces[i].startswith('id:'):
            result['id'] = pieces[i][len('id:'):]
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
    # Semantic results are everything except "Textual Occurrences".
    # We track them so they can be removed from "Textual Occurrences".
    semantic_results = {}
    for (kind, rs) in results.items():
        if kind == 'Textual Occurrences':
            continue

        for result in rs:
            path = result['path']
            for line_info in result['lines']:
                lno = line_info['lno']
                semantic_results[(path, lno)] = True

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

    result_count = [0]
    max_result_count = 1000

    def combine_lines(kind, path, lines1, lines2):
        # Eliminate duplicates and sort by line number.
        dict1 = { l['lno']: l for l in lines1 }
        dict2 = { l['lno']: l for l in lines2 }
        dict1.update(dict2)
        lines = dict1.values()

        # If this is a "Textual Occurrences" result, remove semantic matches.
        if kind == 'Textual Occurrences':
            def keep(l):
                return (path, l['lno']) not in semantic_results
            lines = [ l for l in lines if keep(l) ]

        lines.sort(lambda l1, l2: cmp(l1['lno'], l2['lno']))

        result_count[0] += len(lines)
        if result_count[0] > max_result_count:
            n = result_count[0] - max_result_count
            lines = lines[:-n]
            result_count[0] -= n

        return lines

    def combine(kind, path1r, path2r):
        return {'path': path1r['path'],
                'lines': combine_lines(kind, path1r['path'], path1r['lines'], path2r['lines'])}

    def sort_inner(kind, results):
        m = {}
        for result in results:
            r = combine(kind, m.get(result['path'], result), result)

            # We may have removed everything (due to them being
            # semantic matches). Don't record the path in this case.
            if len(r['lines']):
                m[result['path']] = r

        paths = m.keys()
        paths.sort(sortfunc)

        return [ m[path] for path in paths ]

    # Return results in this order.
    key_precedences = ["IDL", "Definitions", "Assignments", "Uses", "Textual Occurrences", "Declarations"]

    def key_precedence(k):
        for (prec, kind) in enumerate(key_precedences):
            if k.startswith(kind):
                return prec
        return len(key_precedences)

    def key_sort(k1, k2):
        prec1 = key_precedence(k1)
        prec2 = key_precedence(k2)
        if prec1 == prec2:
            return cmp(k1, k2)
        else:
            return cmp(prec1, prec2)

    keys = list(results.keys())
    keys.sort(key_sort)

    r = collections.OrderedDict()
    for k in keys:
        r[k] = sort_inner(k, results[k])
    return r

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

def num_lines(results):
    count = 0
    for k in results:
        for pathspec in results[k]:
            count += len(pathspec['lines'])
    return count

def identifier_search(tree_name, needle, complete, path):
    needle = re.sub(r'\\(.)', r'\1', needle)

    pieces = re.split(r'\.|:', needle)
    if not complete and len(pieces[-1]) < 3:
        return {}

    ids = identifiers.lookup(tree_name, needle, complete)
    print 'IDS', ids
    result = {}
    count = 0
    for (qualified, sym) in ids:
        results = crossrefs.lookup(tree_name, sym)
        for kind in results:
            if path:
                pass
            else:
                k = '%s (%s)' % (kind, qualified)
                result[k] = result.get(k, []) + results[kind]

        count += num_lines(results)
        if count > 1000:
            
            break

    return result

def get_json_search_results(tree_name, query):
    try:
        search_string = query['q'][0]
    except:
        search_string = ''

    try:
        fold_case = query['case'][0] != 'true'
    except:
        fold_case = True

    try:
        regexp = query['regexp'][0] == 'true'
    except:
        regexp = False

    try:
        path_filter = query['path'][0]
    except:
        path_filter = ''

    parsed = parse_search(search_string)

    if path_filter:
        parsed['pathre'] = parse_path_filter(path_filter)

    if regexp:
        if 'default' in parsed:
            del parsed['default']
        if 're' in parsed:
            del parsed['re']
        parsed['re'] = search_string

    if 'default' in parsed and len(parsed['default']) == 0:
        del parsed['default']

    if is_trivial_search(parsed):
        results = {}
        return json.dumps(results)

    title = search_string
    if not title:
        title = 'Files ' + path_filter

    if 'symbol' in parsed:
        symbols = parsed['symbol']
        title = 'Symbol ' + symbols
        results = crossrefs.lookup(tree_name, symbols)
    elif 're' in parsed:
        path = parsed.get('pathre', '.*')
        substr_results = codesearch.search(parsed['re'], fold_case, path, tree_name)
        results = {'Textual Occurrences': substr_results}
    elif 'id' in parsed:
        results = identifier_search(tree_name, parsed['id'], complete=True, path=parsed.get('pathre'))
    elif 'default' in parsed:
        path = parsed.get('pathre', '.*')
        substr_results = codesearch.search(parsed['default'], fold_case, path, tree_name)
        if 'pathre' in parsed:
            file_results = []
            id_results = []
        else:
            file_results = search_files(tree_name, parsed['default'])

        print 'A'
        id_results = identifier_search(tree_name, parsed['default'], complete=False, path=parsed.get('pathre'))
        print 'B'

        results = {'Textual Occurrences': file_results + substr_results}
        results.update(id_results)
    elif 'pathre' in parsed:
        path = parsed['pathre']
        results = {'Textual Occurrences': search_files(tree_name, path)}
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
identifiers.load(config)

class ForkingServer(ForkingMixIn, HTTPServer):
    pass

server_address = ('', 8000)
httpd = ForkingServer(server_address, Handler)
httpd.serve_forever()
