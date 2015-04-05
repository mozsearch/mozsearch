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

class CodeSearch:
    def __init__(self, host, port):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.sock.connect((host, port))
        self.state = 'init'
        self.buffer = ''
        self.matches = []
        self.wait_ready()

    def search(self, needle, fold_case=False, file='.*', repo='.*'):
        print needle
        pattern = re.escape(needle)
        query = {'body': {'fold_case': fold_case, 'line': pattern, 'file': file, 'repo': repo}}
        print 'SEND', json.dumps(query)
        self.state = 'search'
        self.sock.sendall(json.dumps(query) + '\n')
        self.wait_ready()
        matches = self.matches
        self.matches = []
        return matches

    def wait_ready(self):
        while self.state != 'ready':
            input = self.sock.recv(1024)
            print 'RECV', input
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

            self.generate(data)
        elif pathElts[:2] == ['mozilla-central', 'search']:
            query = urlparse.parse_qs(url.query)
            data = json.dumps(codesearch.search(query['q'][0]))
            template = os.path.join(mozSearchPath, 'searchresults-template.html')
            self.generateWithTemplate(data, template)
        elif pathElts[0] == 'crossref':
            template = os.path.join(mozSearchPath, 'crossref-template.html')
            symbol = self.path[len('/crossref/'):].replace('%23', '#')
            data = crossrefs[symbol]
            self.generateWithTemplate(data, template)
        else:
            return SimpleHTTPServer.SimpleHTTPRequestHandler.do_GET(self)

    def generate(self, data):
        self.send_response(200)
        self.send_header("Content-type", "text/html")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()

        self.wfile.write(data)

    def generateWithTemplate(self, data, templateFile):
        template = open(templateFile).read()
        output = template.replace('{{BODY}}', data)

        self.send_response(200)
        self.send_header("Content-type", "text/html")
        self.send_header("Content-Length", str(len(output)))
        self.end_headers()

        self.wfile.write(output)

codesearch = CodeSearch('localhost', 8080)
server_address = ('', 8000)
httpd = BaseHTTPServer.HTTPServer(server_address, Handler)
httpd.serve_forever()
