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

def index_path(tree_name):
    return config['trees'][tree_name]['index_path']

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

def escape_regex(searchString):
    # a version of re.escape that doesn't escape every non-ASCII character,
    # and therefore doesn't mangle utf-8 encoded characters.
    # https://bugzilla.mozilla.org/show_bug.cgi?id=1446220
    return re.sub(r"[(){}\[\].*?|^$\\+-]", r"\\\g<0>", searchString)

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
        elif pieces[i].startswith('text:'):
            result['re'] = escape_regex((' '.join(pieces[i:]))[len('text:'):])
            break
        elif pieces[i].startswith('id:'):
            result['id'] = pieces[i][len('id:'):]
        else:
            result['default'] = escape_regex(' '.join(pieces[i:]))
            break

    return result

def is_trivial_search(parsed):
    if 'symbol' in parsed:
        return False

    for k in parsed:
        if len(parsed[k]) >= 3:
            return False

    return True

class SearchResults(object):
    def __init__(self):
        self.results = []
        self.qualified_results = []

        self.pathre = None
        self.compiled = {}

    def set_path_filter(self, path):
        if not path or path == '.*':
            self.pathre = None
            return

        try:
            self.pathre = re.compile(path, re.IGNORECASE)
        except re.error:
            # In case the pattern is not a valid RE, treat it as literal string.
            self.pathre = re.compile(re.escape(path), re.IGNORECASE)

    def add_results(self, results):
        self.results.append(results)

    def add_qualified_results(self, qual, results, modifier):
        self.qualified_results.append((qual, results, modifier))

    max_count = 1000
    max_work = 750
    path_precedences = ['normal', 'test', 'generated']
    key_precedences = ["Files", "IDL", "Definitions", "Assignments", "Uses", "Declarations", "Textual Occurrences"]

    def categorize_path(self, path):
        def is_test(p):
            if '/unit/' in p:
                return True
            if 'test' not in p:
                return False
            return ('/test/' in p or '/tests/' in p or '/mochitest/' in p or '/unit/' in p or 'testing/' in p or
                    '/jsapi-tests/' in p or '/reftests/' in p or '/reftest/' in p or
                    '/crashtests/' in p or '/crashtest/' in p)

        if '__GENERATED__' in path:
            return 'generated'
        elif is_test(path):
            return 'test'
        else:
            return 'normal'

    def compile_result(self, kind, qual, pathr, line_modifier):
        if qual:
            qkind = '%s (%s)' % (kind, qual)
        else:
            qkind = kind

        path = pathr['path']
        lines = pathr['lines']

        pathkind = self.categorize_path(path)

        if self.pathre and not self.pathre.search(path):
            return

        # compiled is a map {pathkind: {qkind: {path: [(lines, line_modifier)]}}}
        kind_results = self.compiled.setdefault(pathkind, collections.OrderedDict()).setdefault(qkind, {})
        path_results = kind_results.setdefault(path, ([], line_modifier))
        path_results[0].extend(lines)

    def sort_compiled(self):
        count = 0

        line_hash = {}

        result = collections.OrderedDict()
        for pathkind in self.path_precedences:
            for qkind in self.compiled.get(pathkind, []):
                paths = self.compiled[pathkind][qkind].keys()
                paths.sort()
                for path in paths:
                    (lines, line_modifier) = self.compiled[pathkind][qkind][path]
                    lines.sort(key=lambda l: l['lno'])
                    lines_out = []
                    for line in lines:
                        lno = line['lno']
                        key = (path, lno)
                        if key in line_hash:
                            continue
                        line_hash[key] = True
                        if line_modifier:
                            line_modifier(line)
                        lines_out.append(line)
                        count += 1
                        if count == self.max_count:
                            break

                    if lines_out or qkind == 'Files':
                        l = result.setdefault(pathkind, collections.OrderedDict()).setdefault(qkind, [])
                        l.append({'path': path, 'lines': lines_out})
                    if count == self.max_count:
                        break
                if count == self.max_count:
                    break
            if count == self.max_count:
                break

        return result

    def get(self, work_limit):
        # compile_result will categorize each path that it sees.
        # It will build a list of paths indexed by pathkind, qkind.
        # Later I'll iterate over this, remove dupes, sort, and keep the top ones.

        self.qualified_results.sort()
        for kind in self.key_precedences:
            work = 0
            for (qual, results, line_modifier) in self.qualified_results:
                if work > self.max_work and work_limit:
                    log('WORK LIMIT HIT')
                    break
                for pathr in results.get(kind, []):
                    self.compile_result(kind, qual, pathr, line_modifier)
                    work += 1

            for results in self.results:
                for pathr in results.get(kind, []):
                    self.compile_result(kind, None, pathr, None)
                    work += 1

        r = self.sort_compiled()
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

def demangle(sym):
    p = subprocess.Popen(['c++filt', '--no-params', sym], stdout=subprocess.PIPE)
    (stdout, stderr) = p.communicate()
    if not p.returncode:
        return stdout.strip()
    else:
        return sym

def identifier_search(search, tree_name, needle, complete, fold_case):
    needle = re.sub(r'\\(.)', r'\1', needle)

    pieces = re.split(r'\.|::', needle)
    if not complete and len(pieces[-1]) < 3:
        return {}

    def line_modifier(line):
        if 'bounds' in line:
            (start, end) = line['bounds']
            end = start + len(pieces[-1])
            line['bounds'] = [start, end]

    ids = identifiers.lookup(tree_name, needle, complete, fold_case)
    for (i, (qualified, sym)) in enumerate(ids):
        if i > 500:
            break

        q = demangle(sym)
        if q == sym:
            q = qualified

        results = crossrefs.lookup(tree_name, sym)
        search.add_qualified_results(q, results, line_modifier)

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

    search = SearchResults()

    work_limit = False
    hit_timeout = False

    if 'symbol' in parsed:
        search.set_path_filter(parsed.get('pathre'))
        symbols = parsed['symbol']
        title = 'Symbol ' + symbols
        search.add_results(crossrefs.lookup(tree_name, symbols))
    elif 're' in parsed:
        path = parsed.get('pathre', '.*')
        (substr_results, timed_out) = codesearch.search(parsed['re'], fold_case, path, tree_name)
        search.add_results({'Textual Occurrences': substr_results})
        hit_timeout |= timed_out
    elif 'id' in parsed:
        search.set_path_filter(parsed.get('pathre'))
        identifier_search(search, tree_name, parsed['id'], complete=True, fold_case=fold_case)
    elif 'default' in parsed:
        work_limit = True
        path = parsed.get('pathre', '.*')
        (substr_results, timed_out) = codesearch.search(parsed['default'], fold_case, path, tree_name)
        search.add_results({'Textual Occurrences': substr_results})
        hit_timeout |= timed_out
        if 'pathre' not in parsed:
            file_results = search_files(tree_name, parsed['default'])
            search.add_results({'Files': file_results})

            identifier_search(search, tree_name, parsed['default'], complete=False, fold_case=fold_case)
    elif 'pathre' in parsed:
        path = parsed['pathre']
        search.add_results({'Files': search_files(tree_name, path)})
    else:
        assert False
        results = {}

    results = search.get(work_limit)

    results['*title*'] = title
    results['*timedout*'] = hit_timeout
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

        # Warning: many of the branches in this branch condition don't actually
        # get executed, because nginx handles them directly. See scripts/nginx-setup.py
        # for which paths are proxied or handled directly by nginx before you
        # start mucking around in here.

        if not path_elts:
            filename = os.path.join(index_path('mozilla-central'), 'help.html')
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
        elif len(path_elts) >= 2 and path_elts[1] == 'search':
            tree_name = path_elts[0]
            query = urlparse.parse_qs(url.query)
            j = get_json_search_results(tree_name, query)
            if 'json' in self.headers.getheader('Accept', ''):
                self.generate(j, 'application/json')
            else:
                j = j.replace("</", "<\\/").replace("<script", "<\\script").replace("<!", "<\\!")
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

os.chdir(config['mozsearch_path'])

crossrefs.load(config)
codesearch.load(config)
identifiers.load(config)

class ForkingServer(ForkingMixIn, HTTPServer):
    pass

server_address = ('', 8000)
httpd = ForkingServer(server_address, Handler)
httpd.serve_forever()
