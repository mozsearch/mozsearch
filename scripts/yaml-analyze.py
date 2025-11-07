#!/usr/bin/env python3

import sys
import os
import re
import json


def at_escape(text):
    return re.sub("[^A-Za-z0-9_/]", lambda m: "@" + "{:02X}".format(ord(m.group(0))), text)


def to_file_sym(filename):
    return "FILE_" + at_escape(filename)


def to_loc(line, c1, c2):
    return f"{line:05d}:{c1}-{c2}"


class ParseError(Exception):
    def __init__(self, parser, message):
        start = parser.i
        while start > 0:
            if parser.text[start - 1] in ["\r", "\n"]:
                break
            start -= 1

        tlen = len(parser.text)
        end = parser.i
        while end < tlen:
            if parser.text[end] in ["\r", "\n"]:
                break
            end += 1

        column = parser.i - start

        super().__init__(f"{message} at {parser.path}:{parser.lineno}:{parser.column}\n\n" +
                         parser.text[start:end] + "\n" +
                         (" " * column) + "^")


class Parser:
    def __init__(self, local_path, full_path, callback):
        self.local_path = local_path
        self.full_path = full_path
        self.callback = callback

    def parse(self):
        nesting = []
        lineno = 0
        with open(self.full_path, "r") as f:
            for line in f:
                lineno += 1

                line = line.rstrip()
                m = re.match(r"^([ \t-]*)([^#]+?):([ \t]+|$)", line)
                if m:
                    depth = len(m.group(1))
                    while len(nesting) and depth <= nesting[-1][1]:
                        last = nesting.pop()
                        self.callback.on_key_end(last[0], last[1], last[2], lineno)
                    key = m.group(2)
                    nesting.append([key, depth, lineno])
                    continue

                m = re.match(r"^([ \t]*)[^ \t]", line)
                if m:
                    depth = len(m.group(1))
                    while len(nesting) and depth <= nesting[-1][1]:
                        last = nesting.pop()
                        self.callback.on_key_end(last[0], last[1], last[2], lineno)

        while len(nesting):
            last = nesting.pop()
            self.callback.on_key_end(last[0], last[1], last[2], lineno)


class AnalysisWriter:
    def __init__(self, local_path, analysis_path):
        self.test_dir = os.path.dirname(local_path)
        self.analysis_path = analysis_path
        self.items = []

        self.items.append({
            "loc": "00001:0",
            "target": 1,
            "kind": "def",
            "pretty": local_path,
            "sym": to_file_sym(local_path),
        })

    def write(self):
        with open(self.analysis_path, "w") as f:
            for item in sorted(self.items, key=lambda x: x["loc"]):
                print(json.dumps(item), file=f)

    def on_key_end(self, key, depth, first_line, last_line):
        self.items.append({
            "loc": to_loc(first_line, depth, depth + len(key)),
            "source": 1,
            "syntax": "key",
            "pretty": key,
            "sym": "YAMLKEY_" + at_escape(key),
            "nestingRange": f"{first_line}:{depth}-{last_line}:0",
        })


def analyze(local_path, files_root, analysis_root):
    yaml_path = os.path.join(files_root, local_path)
    analysis_path = os.path.join(analysis_root, local_path)

    w = AnalysisWriter(local_path, analysis_path)
    p = Parser(local_path, yaml_path, w)
    try:
        p.parse()
    except ParseError as e:
        return
    w.write()


index_root = sys.argv[1]
files_root = sys.argv[2]
analysis_root = sys.argv[3]

for local_path in sys.stdin:
    local_path = local_path.strip()
    analyze(local_path, files_root, analysis_root)
