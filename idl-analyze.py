import sys
import xpidl
import os.path
import json

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

def read_cpp_analysis(fname):
    base = os.path.basename(fname)
    (idlName, suffix) = os.path.splitext(base)
    headerName = idlName + '.h'
    p = os.path.join(indexRoot, 'analysis', '__GENERATED__', 'dist', 'include', headerName)
    try:
        lines = open(p).readlines()
    except IOError, e:
        return None
    decls = []
    for line in lines:
        j = json.loads(line.strip())
        if 'target' in j and j['kind'] == 'decl' and j['sym'].startswith('_Z'):
            decls.append(j['sym'])
    return decls

def handle_interface(iface):
    (lineno, colno) = find_line_column(text, iface.name, iface.location._lexpos)
    mangled = 'T_' + iface.name

    # Source
    j = {
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.name)),
        'source': 1,
        'pretty': 'IDL class %s' % iface.name,
        'sym': mangled + ('' if iface.attributes.scriptable else ',#' + iface.name),
    }
    print json.dumps(j)

    # C++ target
    j = {
        'loc': '%d:%d' % (lineno, colno),
        'target': 1,
        'kind': 'idl',
        'sym': mangled,
    }
    print json.dumps(j)

    if iface.attributes.scriptable:
        # JS target
        j = {
            'loc': '%d:%d' % (lineno, colno),
            'target': 1,
            'kind': 'idl',
            'sym': '#' + iface.name,
        }
        print json.dumps(j)
    
    (lineno, colno) = find_line_column(text, iface.base, iface.location._lexpos)
    mangled = 'T_' + iface.base

    # Base source
    j = {
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.base)),
        'source': 1,
        'pretty': 'IDL class %s' % iface.base,
        'sym': mangled + ',#' + iface.base,
    }
    print json.dumps(j)

    #print p.name
    #print 'BASE', p.base
    for m in iface.members:
        name = getattr(m, 'name', '')
        (lineno, colno) = find_line_column(text, name, m.location._lexpos)

        # Want to deal with attributes like noscript, as well as ConstMember

        if isinstance(m, xpidl.Method):
            mangled = analysis.pop(0)
            # C++ target
            j = {
                'loc': '%d:%d' % (lineno, colno),
                'target': 1,
                'kind': 'idl',
                'sym': mangled,
            }
            print json.dumps(j)
            
            # Source
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'source': 1,
                'pretty': 'IDL method %s' % m.name,
                'sym': mangled + ('' if m.noscript else ',#' + m.name),
            }
            print json.dumps(j)

            if not m.noscript:
                # JS target
                j = {
                    'loc': '%d:%d' % (lineno, colno),
                    'target': 1,
                    'kind': 'idl',
                    'sym': '#' + m.name,
                }
                print json.dumps(j)

        elif isinstance(m, xpidl.Attribute):
            if not m.noscript:
                # JS target
                j = {
                    'loc': '%d:%d' % (lineno, colno),
                    'target': 1,
                    'kind': 'idl',
                    'sym': '#' + m.name,
                }
                print json.dumps(j)

            mangled_getter = analysis.pop(0)

            # C++ target (getter)
            j = {
                'loc': '%d:%d' % (lineno, colno),
                'target': 1,
                'kind': 'idl',
                'sym': mangled_getter,
            }
            print json.dumps(j)

            if not m.readonly:
                mangled_setter = analysis.pop(0)

                # C++ target (setter)
                j = {
                    'loc': '%d:%d' % (lineno, colno),
                    'target': 1,
                    'kind': 'idl',
                    'sym': mangled_setter,
                }
                print json.dumps(j)

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
            print json.dumps(j)

        elif isinstance(m, xpidl.ConstMember):
            # No C++ support until clang-plugin supports it.

            # JS target
            j = {
                'loc': '%d:%d' % (lineno, colno),
                'target': 1,
                'kind': 'idl',
                'sym': '#' + m.name,
            }
            print json.dumps(j)

            # JS source
            j = {
                'loc': '%d:%d-%d' % (lineno, colno, colno + len(m.name)),
                'source': 1,
                'pretty': 'IDL constant %s' % m.name,
                'sym': '#' + m.name,
            }
            print json.dumps(j)

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
    p = xpidl.IDLParser(outputdir='/tmp')
    try:
        r = p.parse(text, filename=fname)
    except xpidl.IDLError, e:
        print >>sys.stderr, 'Syntax error in IDL', fname
        raise e
        sys.exit(1)
    for p in r.productions:
        if isinstance(p, xpidl.Interface):
            handle_interface(p)
