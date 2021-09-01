#!/usr/bin/env python2

from __future__ import absolute_import
from __future__ import print_function
import sys
import xpidl
import os.path
import json
import re

def find_line_column(text, token, pos):
    while text[pos : pos + len(token)] != token:
        if text[pos] == '\n':
            return (0, 0)

        pos = text.find(' ', pos) + 1

    line = 0
    while pos > linebreaks[line]:
        line += 1

    line += 1

    col = 0
    pos -= 1
    while pos > 0 and text[pos] != '\n':
        col += 1
        pos -= 1

    return (line, col)

def parse_mangled(mangled):
    def parse_inner(idents, inner):
        if inner[0] == 'E':
            return True

        m = re.match(r'([0-9]+)([a-zA-Z0-9_]+)', inner)
        if not m:
            return False

        length = int(m.group(1))
        idents.append(m.group(2)[:length])

        return parse_inner(idents, m.group(2)[length:])

    if mangled[:3] != '_ZN':
        return

    idents = []
    if not parse_inner(idents, mangled[3:]):
        return

    return idents

def read_cpp_analysis(fname):
    base = os.path.basename(fname)
    (idlName, suffix) = os.path.splitext(base)
    headerName = idlName + '.h'
    p = os.path.join(indexRoot, 'analysis', '__GENERATED__', 'dist', 'include', headerName)
    try:
        lines = open(p).readlines()
    except IOError as e:
        return None
    decls = {}
    for line in lines:
        try:
            j = json.loads(line.strip())
        except ValueError as e:
            print('Syntax error in JSON file', p, line.strip(), file=sys.stderr)
            raise e
        # Inline method definitions and pure virtual method declarations
        # will both be reported as definitions by the C++ indexer without a
        # declaration, so we need to accept both decls and defs.
        if 'target' in j and (j['kind'] == 'decl' or j['kind'] == 'def') and j['sym'].startswith('_Z'):
            idents = parse_mangled(j['sym'])
            if idents and len(idents) == 2:
                decls.setdefault(idents[0], {})[idents[1]] = j['sym']
    return decls

def method_name(m):
    if m.binaryname:
        return m.binaryname
    return m.name[0].capitalize() + m.name[1:]

def getter_name(attr):
    if attr.binaryname:
        return 'Get' + attr.binaryname
    return 'Get' + attr.name[0].capitalize() + attr.name[1:]

def setter_name(attr):
    if attr.binaryname:
        return 'Set' + attr.binaryname
    return 'Set' + attr.name[0].capitalize() + attr.name[1:]

def handle_interface(analysis, iface):
    (lineno, colno) = find_line_column(text, iface.name, iface.location._lexpos)
    mangled = 'T_' + iface.name

    # Source
    j = {
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.name)),
        'source': 1,
        'pretty': 'IDL class %s' % iface.name,
        'sym': mangled + ('' if iface.attributes.scriptable else ',#' + iface.name),
    }
    print(json.dumps(j))

    # C++ target
    j = {
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.name)),
        'target': 1,
        'kind': 'idl',
        'sym': mangled,
    }
    print(json.dumps(j))

    if iface.attributes.scriptable:
        # JS target
        j = {
            'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.name)),
            'target': 1,
            'kind': 'idl',
            'sym': '#' + iface.name,
        }
        print(json.dumps(j))

    if iface.base:
        (lineno, colno) = find_line_column(text, iface.base, iface.location._lexpos)
        mangled = 'T_' + iface.base

        # Base source
        j = {
            'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.base)),
            'source': 1,
            'pretty': 'IDL class %s' % iface.base,
            'sym': mangled + ',#' + iface.base,
        }
        print(json.dumps(j))

    #print p.name
    #print 'BASE', p.base
    for m in iface.members:
        name = getattr(m, 'name', '')
        (lineno, colno) = find_line_column(text, name, m.location._lexpos)

        # Want to deal with attributes like noscript, as well as ConstMember

        if isinstance(m, xpidl.Method):
            mangled = analysis[method_name(m)]
            # C++ target
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'target': 1,
                'kind': 'idl',
                'sym': mangled,
            }
            print(json.dumps(j))

            # Source
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'source': 1,
                'pretty': 'IDL method %s' % m.name,
                'sym': mangled + ('' if m.noscript else ',#' + m.name),
            }
            print(json.dumps(j))

            if not m.noscript:
                # JS target
                j = {
                    'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                    'target': 1,
                    'kind': 'idl',
                    'sym': '#' + m.name,
                }
                print(json.dumps(j))

        elif isinstance(m, xpidl.Attribute):
            if not m.noscript:
                # JS target
                j = {
                    'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                    'target': 1,
                    'kind': 'idl',
                    'sym': '#' + m.name,
                }
                print(json.dumps(j))

            mangled_getter = analysis[getter_name(m)]

            # C++ target (getter)
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'target': 1,
                'kind': 'idl',
                'sym': mangled_getter,
            }
            print(json.dumps(j))

            if not m.readonly:
                mangled_setter = analysis[setter_name(m)]

                # C++ target (setter)
                j = {
                    'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                    'target': 1,
                    'kind': 'idl',
                    'sym': mangled_setter,
                }
                print(json.dumps(j))

            # Source
            sym = mangled_getter
            if not m.readonly:
                sym += ',' + mangled_setter
            if not m.noscript:
                sym += ',#' + m.name
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'source': 1,
                'pretty': 'IDL attribute %s' % m.name,
                'sym': sym,
            }
            print(json.dumps(j))

        elif isinstance(m, xpidl.ConstMember):
            # No C++ support until clang-plugin supports it.

            # JS target
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'target': 1,
                'kind': 'idl',
                'sym': '#' + m.name,
            }
            print(json.dumps(j))

            # JS source
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'source': 1,
                'pretty': 'IDL constant %s' % m.name,
                'sym': '#' + m.name,
            }
            print(json.dumps(j))

indexRoot = sys.argv[1]
fname = sys.argv[2]

text = open(fname).read()
analysis = read_cpp_analysis(fname)

linebreaks = []
lines = text.split('\n')
cur = 0
for l in lines:
    cur += len(l) + 1
    linebreaks.append(cur)

if analysis:
    # compatibility before and after bug 1633156
    import inspect
    initMethod = [obj for (name, obj) in inspect.getmembers(xpidl.IDLParser) if name == '__init__'][0]
    if 'outputdir' in inspect.getargspec(initMethod).args:
        p = xpidl.IDLParser(outputdir='/tmp')
    else:
        p = xpidl.IDLParser()

    try:
        r = p.parse(text, filename=fname)
    except xpidl.IDLError as e:
        print('Syntax error in IDL', fname, file=sys.stderr)
        raise e
        sys.exit(1)
    for p in r.productions:
        if isinstance(p, xpidl.Interface):
            handle_interface(analysis.get(p.name, {}), p)
