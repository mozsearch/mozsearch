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


def to_glean_key(nesting):
    names = list(map(lambda x: x[0], nesting))
    names = list(map(lambda x: x.replace(".", "_"), names))
    return "::".join(names)


def to_camel_case(s):
    s = re.sub(r"_([a-z])", lambda x: x[1].upper(), s)
    return s


def to_glean_js_sym(nesting):
    key0 = to_camel_case(nesting[0][0].replace(".", "_"))
    key1 = to_camel_case(nesting[1][0].replace(".", "_"))
    return f"Glean.{key0}#{key1}"


class Parser:
    def __init__(self, local_path, full_path, callback, glean_sym_map):
        self.local_path = local_path
        self.full_path = full_path
        self.callback = callback
        self.glean_sym_map = glean_sym_map
        self.is_glean_metrics = self.local_path.endswith("metrics.yaml")

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

                    if self.is_glean_metrics:
                        self.handle_glean_metrics(nesting, depth, lineno)
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

    def handle_glean_metrics(self, nesting, depth, lineno):
        if len(nesting) == 2:
            glean_key = to_glean_key(nesting)
            if glean_key in self.glean_sym_map:
                self.callback.on_glean_binding(
                    nesting[-1][0],
                    glean_key,
                    depth,
                    lineno,
                    self.glean_sym_map[glean_key],
                    to_glean_js_sym(nesting),
                    "field",
                    "member"
                )
            return

        if len(nesting) == 3 and nesting[2][0] == "extra_keys":
            glean_key = to_glean_key(nesting)
            if glean_key in self.glean_sym_map:
                self.callback.on_glean_binding(
                    nesting[-1][0],
                    glean_key,
                    depth,
                    lineno,
                    self.glean_sym_map[glean_key],
                    None,
                    "field",
                    "class"
                )
            return

        if len(nesting) == 4 and nesting[2][0] == "extra_keys":
            glean_key = to_glean_key(nesting)
            if glean_key in self.glean_sym_map:
                self.callback.on_glean_binding(
                    nesting[-1][0],
                    glean_key,
                    depth,
                    lineno,
                    self.glean_sym_map[glean_key],
                    None,
                    "field",
                    "member"
                )
            return


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

    def on_glean_binding(self, key, glean_key, depth, lineno, cpp_sym,
                         js_sym,
                         structured_kind, slot_kind):
        self.items.append({
            "loc": to_loc(lineno, depth, depth + len(key)),
            "source": 1,
            "syntax": "def",
            "pretty": glean_key,
            "sym": "GLEAN_" + glean_key,
        })
        self.items.append({
            "loc": to_loc(lineno, depth, depth + len(key)),
            "target": 1,
            "kind": "glean",
            "pretty": glean_key,
            "sym": "GLEAN_" + glean_key,
        })
        slots = []
        slots.append({
            "slotKind": slot_kind,
            "slotLang": "cpp",
            "ownerLang": "glean",
            "sym": cpp_sym,
            "implKind": "binding",
        })
        if js_sym is not None:
            slots.append({
                "slotKind": slot_kind,
                "slotLang": "js",
                "ownerLang": "glean",
                "sym": js_sym,
                "implKind": "binding",
            })
        self.items.append({
            "loc": to_loc(lineno, depth, depth + len(key)),
            "structured": 1,
            "pretty": glean_key,
            "sym": "GLEAN_" + glean_key,
            "kind": structured_kind,
            "implKind": "glean",
            "bindingSlots": slots,
        })

    def on_key_end(self, key, depth, first_line, last_line):
        self.items.append({
            "loc": to_loc(first_line, depth, depth + len(key)),
            "source": 1,
            "syntax": "key",
            "pretty": key,
            "sym": "YAMLKEY_" + at_escape(key),
            "nestingRange": f"{first_line}:{depth}-{last_line}:0",
        })


def analyze(local_path, files_root, analysis_root, sym_map):
    yaml_path = os.path.join(files_root, local_path)
    analysis_path = os.path.join(analysis_root, local_path)

    w = AnalysisWriter(local_path, analysis_path)
    p = Parser(local_path, yaml_path, w, sym_map)
    try:
        p.parse()
    except ParseError as e:
        return
    w.write()


def to_snake_case(s):
    s = re.sub(r"^[A-Z]", lambda x: x[0].lower(), s)
    s = re.sub(r"[A-Z]", lambda x: "_" + x[0].lower(), s)
    return s


def collect_glean_cpp_binding_one(path, sym_map):
    with open(path, "r") as f:
        for line in f:
            if '"structured":1' not in line:
                continue
            data = json.loads(line.strip())
            if not data["pretty"].startswith("mozilla::glean::"):
                continue

            key = data["pretty"].replace("mozilla::glean::", "")
            if data["kind"] == "struct" and data["sym"].endswith("Extra"):
                struct_pretty = data["pretty"]

                prefix, component = key.rsplit("::", 1)
                component = re.sub('Extra$', '', component)
                component = to_snake_case(component)
                struct_key = prefix + "::" + component + "::extra_keys"
                sym_map[struct_key] = data["sym"]

                for field in data["fields"]:
                    name = field["pretty"].replace(struct_pretty + "::", "")
                    field_key = struct_key + "::" + to_snake_case(name)
                    sym_map[field_key] = field["sym"]
            else:
                sym_map[key] = data["sym"]


def collect_glean_cpp_bindings(analysis_path, binding_path):
    sym_map = {}

    if binding_path == "null":
        binding_path = os.path.join("__GENERATED__", "toolkit", "components", "glean")
    binding_root = os.path.join(analysis_path, binding_path)
    if not os.path.exists(binding_root):
        return sym_map

    for name in os.listdir(binding_root):
        if not name.endswith("Metrics.h"):
            continue
        collect_glean_cpp_binding_one(os.path.join(binding_root, name),
                                      sym_map)

    return sym_map


index_root = sys.argv[1]
files_root = sys.argv[2]
analysis_root = sys.argv[3]
binding_path = sys.argv[4]

glean_sym_map = collect_glean_cpp_bindings(analysis_root, binding_path)

for local_path in sys.stdin:
    local_path = local_path.strip()
    analyze(local_path, files_root, analysis_root, glean_sym_map)
