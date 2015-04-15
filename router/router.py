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

mozSearchPath = sys.argv[1]
indexPath = sys.argv[2]

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

    def collateMatches(self, matches):
        paths = {}
        for m in matches:
            paths.setdefault(m['path'], []).append({'lno': m['lno'], 'line': m['line'].strip()})
        results = [ {'path': p, 'icon': '', 'lines': paths[p]} for p in paths ]
        return {"default": results}

    def search(self, needle, fold_case=False, file='.*', repo='.*'):
        pattern = re.escape(needle)
        query = {'body': {'fold_case': fold_case, 'line': pattern, 'file': file, 'repo': repo}}
        self.state = 'search'
        self.sock.sendall(json.dumps(query) + '\n')
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
            pass
        else:
            raise 'Unknown opcode %s' % j['opcode']

def get_json_search_results(query):
    searchString = query['q'][0]
    if searchString.startswith('symbol:'):
        symbol = searchString[len('symbol:'):].strip().replace('.', '#')
        return crossrefs.get(symbol, "[]")
    elif searchString.startswith('path:'):
        path = searchString[len('path:'):]
        if len(path) < 3:
            return json.dumps({})
        results = []
        for f in allFiles:
            if path in f:
                results.append({'path': f, 'icon': '', 'lines': []})
        results = results[:1000]
        return json.dumps({"default": results})
    else:
        return json.dumps(codesearch.search(searchString))

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
                data = open(filename).read()

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

codesearch = CodeSearch('localhost', 8080)
server_address = ('', 8000)
httpd = BaseHTTPServer.HTTPServer(server_address, Handler)
httpd.serve_forever()
