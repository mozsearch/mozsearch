import sys
import xpidl
import os.path
import json
import re

def find_line_column(text, token, pos):
    while text[pos : pos + len(token)] != token:
        if text[pos] == '\n':
            return (0, 0)

        pos += 1

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
    except IOError, e:
        sys.exit(0)

    methods = {}
    enums = []
    for line in lines:
        j = json.loads(line.strip())
        if 'target' in j and j['kind'] in ('decl', 'def'):
            if j['sym'].startswith('_Z'):
                idents = parse_mangled(j['sym'])
                if idents and len(idents) == 2:
                    methods.setdefault(idents[0], {})[idents[1]] = j['sym']
            elif j['sym'].startswith('E_'):
                enums.append(j['sym'])

    return (methods, enums)

def find_enum(enums, name):
    for e in enums:
        if e.endswith(name):
            return e

def cxx_method_name(m):
    if m.binaryname:
        return m.binaryname
    return m.name[0].capitalize() + m.name[1:]

def cxx_getter_name(attr):
    if attr.binaryname:
        return 'Get' + attr.binaryname
    return 'Get' + attr.name[0].capitalize() + attr.name[1:]

def cxx_setter_name(attr):
    if attr.binaryname:
        return 'Set' + attr.binaryname
    return 'Set' + attr.name[0].capitalize() + attr.name[1:]

def if_true(scriptable, name):
    if scriptable:
        return [name]
    else:
        return []

def source(lineno, colno, kind, name, syms):
    j = {
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(name)),
        'source': 1,
        'pretty': 'IDL %s %s' % (kind, name),
        'sym': ','.join(syms),
    }
    print json.dumps(j)

def target(lineno, colno, kind, name, sym):
    j = {
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(name)),
        'target': 1,
        'kind': 'idl',
        'pretty': 'IDL %s %s' % (kind, name),
        'sym': sym,
    }
    print json.dumps(j)

def handle_interface(methods, enums, iface):
    (lineno, colno) = find_line_column(text, iface.name, iface.location._lexpos)
    mangled = 'T_' + iface.name

    # Source
    source(lineno, colno, 'class', iface.name,
           [mangled] + if_true(iface.attributes.scriptable, '#' + iface.name))

    # C++ target
    target(lineno, colno, 'class', iface.name, mangled)

    # JS target
    if iface.attributes.scriptable:
        target(lineno, colno, 'class', iface.name, '#' + iface.name)

    if iface.base:
        (lineno, colno) = find_line_column(text, iface.base, iface.location._lexpos)
        mangled = 'T_' + iface.base

        # Base source
        source(lineno, colno, 'class', iface.base,
               [mangled] + if_true(iface.attributes.scriptable, '#' + iface.base))

        target(lineno, colno, 'class', iface.base, mangled)
        if iface.attributes.scriptable:
            target(lineno, colno, 'class', iface.base, '#' + iface.base)

    #print p.name
    #print 'BASE', p.base
    for m in iface.members:
        name = getattr(m, 'name', '')
        (lineno, colno) = find_line_column(text, name, m.location._lexpos)

        # Want to deal with attributes like noscript, as well as ConstMember

        if isinstance(m, xpidl.Method):
            mangled = methods[cxx_method_name(m)]

            # Source
            source(lineno, colno, 'method', m.name,
                   [mangled] + if_true(not m.noscript, '#' + m.name))

            # C++ target
            target(lineno, colno, 'method', m.name, mangled)

            # JS target
            if not m.noscript:
                target(lineno, colno, 'method', m.name, '#' + m.name)

        elif isinstance(m, xpidl.Attribute):
            mangled_getter = methods[cxx_getter_name(m)]
            mangled_setter = methods.get(cxx_setter_name(m))

            source(lineno, colno, 'attribute', m.name,
                   [mangled_getter] + if_true(not m.readonly, mangled_setter) +
                   if_true(not m.noscript, '#' + m.name))

            target(lineno, colno, 'attribute', m.name, mangled_getter)
            if not m.readonly:
                target(lineno, colno, 'attribute', m.name, mangled_setter)
            if not m.noscript:
                target(lineno, colno, 'attribute', m.name, '#' + m.name)

        elif isinstance(m, xpidl.ConstMember):
            mangled = find_enum(enums, m.name)

            # JS source
            source(lineno, colno, 'constant', m.name, ['#' + m.name] + if_true(mangled, mangled))

            # JS target
            target(lineno, colno, 'constant', m.name, '#' + m.name)

            # C++ target
            if mangled:
                target(lineno, colno, 'constant', m.name, mangled)

indexRoot = sys.argv[1]
fname = sys.argv[2]

text = open(fname).read()
(methods, enums) = read_cpp_analysis(fname)

linebreaks = []
lines = text.split('\n')
cur = 0
for l in lines:
    cur += len(l) + 1
    linebreaks.append(cur)

if methods:
    p = xpidl.IDLParser(outputdir='/tmp')
    try:
        r = p.parse(text, filename=fname)
    except xpidl.IDLError, e:
        print >>sys.stderr, 'Syntax error in IDL', fname
        raise e
        sys.exit(1)
    for p in r.productions:
        if isinstance(p, xpidl.Interface):
            handle_interface(methods.get(p.name, {}), enums, p)
