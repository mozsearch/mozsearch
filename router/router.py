import BaseHTTPServer
import SimpleHTTPServer
import urllib
import urlparse
import sys
import os
import os.path
import socket
import json
import re
import subprocess

mozSearchPath = sys.argv[1]
indexPath = sys.argv[2]

do_codesearch = True
for opt in sys.argv[3:]:
    if opt == '--no-codesearch':
        do_codesearch = False

crossrefs = {}

lines = open(os.path.join(indexPath, 'crossref')).readlines()
key = None
for line in lines:
    if key == None:
        key = line.strip()
    else:
        value = line.strip()
        crossrefs[key] = value
        key = None

allFiles = open(os.path.join(indexPath, 'all-files')).readlines()

class CodeSearch:
    def __init__(self, host, port):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.connect((host, port))
        self.state = 'init'
        self.buffer = ''
        self.matches = []
        self.wait_ready()
        self.query = None

    def collateMatches(self, matches):
        paths = {}
        for m in matches:
            paths.setdefault(m['path'], []).append({
                'lno': m['lno'],
                'bounds': m['bounds'],
                'line': m['line']
            })
        results = [ {'path': p, 'icon': '', 'lines': paths[p]} for p in paths ]
        return results

    def search(self, pattern, fold_case=True, file='.*', repo='.*'):
        query = {'body': {'fold_case': fold_case, 'line': pattern, 'file': file, 'repo': repo}}
        self.query = json.dumps(query)
        self.state = 'search'
        self.sock.sendall(self.query + '\n')
        self.wait_ready()
        matches = self.collateMatches(self.matches)
        self.matches = []
        return matches

    def wait_ready(self):
        while self.state != 'ready':
            input = self.sock.recv(1024)
            self.buffer += input
            self.handle_input()

    def handle_input(self):
        try:
            pos = self.buffer.index('\n')
        except:
            pos = -1

        if pos >= 0:
            line = self.buffer[:pos]
            self.buffer = self.buffer[pos + 1:]
            self.handle_line(line)
            self.handle_input()

    def handle_line(self, line):
        j = json.loads(line)
        if j['opcode'] == 'match':
            self.matches.append(j['body'])
        elif j['opcode'] == 'ready':
            self.state = 'ready'
        elif j['opcode'] == 'done':
            if j.get('body', {}).get('why') == 'timeout':
                print 'Timeout', self.query
        else:
            raise 'Unknown opcode %s' % j['opcode']

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

def sort_results(results):
    def is_test(p):
        return '/test/' in p or '/tests/' in p or '/mochitest/' in p or '/unit/' in p or 'testing/' in p

    # neg if p1 is before p2
    def sortfunc(p1, p2):
        t1 = is_test(p1)
        t2 = is_test(p2)
        r = cmp(p1, p2)
        if t1 and not t2:
            r += 10000
        elif t2 and not t1:
            r -= 10000
        return r

    def sort_inner(results):
        m = {}
        for result in results:
            if result['path'] not in m or len(result['lines']):
                m[result['path']] = result

        paths = m.keys()
        paths.sort(sortfunc)

        return [ m[path] for path in paths ]

    return { kind: sort_inner(res) for kind, res in results.items() }

def search_files(path):
    pathFile = os.path.join(indexPath, 'all-files')
    try:
        results = subprocess.check_output(['grep', '-Ei', path, pathFile])
    except:
        return []
    results = results.strip().split('\n')
    results = [ {'path': f[1:], 'lines': []} for f in results ]
    return results[:1000]

def get_json_search_results(query):
    searchString = query['q'][0]
    try:
        foldCase = query['case'][0] != 'true'
    except:
        foldCase = True

    parsed = parse_search(searchString)

    if 'symbol' in parsed:
        # FIXME: Need to deal with path here
        symbol = parsed['symbol']
        results = json.loads(crossrefs.get(symbol, "{}"))
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
    print json.dumps(results)
    return json.dumps(results)

class Handler(SimpleHTTPServer.SimpleHTTPRequestHandler):
    def do_GET(self):
        url = urlparse.urlparse(self.path)
        pathElts = url.path.split('/')

        # Strip any extra slashes.
        pathElts = [ elt for elt in pathElts if elt != '' ]

        if pathElts[:2] == ['mozilla-central', 'source']:
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
                title = query['q'][0]
                j = j.replace("/script", "\\/script")
                template = os.path.join(indexPath, 'templates/search.html')
                self.generateWithTemplate({'{{BODY}}': j, '{{TITLE}}': title}, template)
        elif pathElts[:2] == ['mozilla-central', 'define']:
            query = urlparse.parse_qs(url.query)
            symbol = query['q'][0]
            print symbol
            results = json.loads(crossrefs.get(symbol, "{}"))
            print results
            definition = results['Definitions'][0]
            filename = definition['path']
            lineno = definition['lines'][0]['lno']
            url = '/mozilla-central/source/' + filename + '#' + str(lineno)

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

if do_codesearch:
    codesearch = CodeSearch('localhost', 8080)

server_address = ('', 8000)
httpd = BaseHTTPServer.HTTPServer(server_address, Handler)
httpd.serve_forever()
