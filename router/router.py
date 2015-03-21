import BaseHTTPServer
import SimpleHTTPServer
import urllib
import sys
import os
import os.path

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

class Handler(SimpleHTTPServer.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith('/file/'):
            filename = os.path.join(indexPath, 'file', self.path[len('/file/'):])
            data = open(filename).read()
            template = os.path.join(mozSearchPath, 'file-template.html')
            self.generateWithTemplate(data, template)
        elif self.path.startswith('/crossref/'):
            template = os.path.join(mozSearchPath, 'crossref-template.html')
            symbol = self.path[len('/crossref/'):].replace('%23', '#')
            data = crossrefs[symbol]
            self.generateWithTemplate(data, template)
        else:
            return SimpleHTTPServer.SimpleHTTPRequestHandler.do_GET(self)

    def generateWithTemplate(self, data, templateFile):
        template = open(templateFile).read()
        output = template.replace('{{BODY}}', data)

        self.send_response(200)
        self.send_header("Content-type", "text/html")
        self.send_header("Content-Length", str(len(output)))
        self.end_headers()

        self.wfile.write(output)

def run():
    server_address = ('', 8000)
    httpd = BaseHTTPServer.HTTPServer(server_address, Handler)
    httpd.serve_forever()

run()
