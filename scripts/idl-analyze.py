#!/usr/bin/env python3
#
# ## Overview / Purpose ##
#
# Currently, this file is responsible for using the downloaded python XPIDL
# parser to produce analysis records for XPIDL files (with an `idl` extension).
# The XPIDL file is parsed and used to cross-reference the C++ analysis records
# produced from the C++ analysis pass of the generated XPIDL C++ bindings in
# order to establish the relationship between the IDL file and the bindingss.
#
# This file is also aware of the JS bindings but because of the limitations of
# our JS language analyzers at this time, the JS bindings will generally result
# in an immense number of false positives.  Specifically, for an XPIDL method
# "foo", we will generate a symbol `#foo` which will match every JS method or
# variable named "foo".
#
# It is our hope that in the future this script can be superseded by having the
# in-tree XPIDL binding generator (and source of the `xpidl` import) directly
# generate analysis records with meta-data providing support for a new semantic
# linker step proposed at https://bugzilla.mozilla.org/show_bug.cgi?id=1727789
# to allow for a more
#
# ## XPIDL symbols versus C++, JS Symbols and router.py ##
#
# Before the "structured" branch landed, our IDL files (XPIDL, IPDL) never had
# symbols that corresponded directly to the IDL file itself.  Instead, the IDL
# analyzers would try and find the relevant C++ symbols and guess the relevant
# JS symbols and then reference them in the file.  This was necessary because
# searchfox never had any concept of relationship with symbols, it only had the
# ability to associate symbols with a token, grouping by the "pretty" identifier
# associated with the symbols.
#
# The "structured" functionality has now enabled us to convey the explicit
# relationships between symbols.  This comment is being written as part of an
# effort to transition the context-menu's "source" records from pre-conflating
# the C++ and JS symbols to instead using explicit `XPIDL_foo` symbols which
# have explicit relationship to the binding symbols.
#
# However, we can only modernize the "source" records right now because the UI
# breakdown is:
# - "source" records power the context menu in source listings.
# - "target" records power the crossref database which router.py displays.
#
# The new `pipeline-server.rs` and its "query" endpoint have been intentionally
# designed to be able to understand and handle following these relationship
# edges.
#
# `router.py` has also been augmented to:
# - Process the "slotOwner" "meta" field by exposing its def(s) as "IDL".  This
#   means that when looking at a C++ method/getter/setter, the IDL definition
#   will be exposed as "IDL".  Note that we will not traverse the IDL def, so
#   in order to see the JS method/getter/setter, the user will need to switch
# - Process the "bindingSlots"
# traverse the "slotOwner"
# and "bindingSlots" "meta" fields to maintain its behavior prior to this change
# but no attempt is made to enhance the "search" endpoint to opt out of this
# behavior or

from __future__ import absolute_import
from __future__ import print_function
import sys
import xpidl
import os.path
import json
import re
import sys

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
    methods = {}
    enums = []
    for line in lines:
        try:
            j = json.loads(line.strip())
        except ValueError as e:
            print('Syntax error in JSON file', p, line.strip(), file=sys.stderr)
            raise e
        # Inline method definitions and pure virtual method declarations
        # will both be reported as definitions by the C++ indexer without a
        # declaration, so we need to accept both decls and defs.
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

def cpp_method_name(m):
    '''Return the C++ pretty name for this method per binaryname or capitalization.'''
    if m.binaryname:
        return m.binaryname
    return m.name[0].capitalize() + m.name[1:]

def cpp_getter_name(attr):
    '''Return the C++ pretty name for this getter per binaryname or capitalization.'''
    if attr.binaryname:
        return 'Get' + attr.binaryname
    return 'Get' + attr.name[0].capitalize() + attr.name[1:]

def cpp_setter_name(attr):
    '''Return the C++ pretty name for this setter per binaryname or capitalization.'''
    if attr.binaryname:
        return 'Set' + attr.binaryname
    return 'Set' + attr.name[0].capitalize() + attr.name[1:]

def emit_record(o, variations=None):
    '''Emit a single record or a number of optional variations of a record.

    If no variations are provided, the base object is emitted as JSON.

    If a variations array is provided, any non-None entries are applied to the
    base dictionary and emitted as JSON.
    '''
    if variations is None:
        print(json.dumps(o))
        return

    for mods in variations:
        if mods is None:
            continue
        cur = o.copy()
        cur.update(mods)
        print(json.dumps(cur))

def handle_interface(methods, enums, iface):
    '''Derives analysis records for each provided interface.

    '''
    (lineno, colno) = find_line_column(text, iface.name, iface.location._lexpos)
    iface_cpp_sym = 'T_' + iface.name
    iface_idl_sym = f'XPIDL_{ iface.name }'

    iface_loc = '%d:%d-%d' % (lineno, colno, colno + len(iface.name))

    # structured record will be emitted after processing all the members, but
    # we build up any complex sub-structures here.
    iface_slots = [
        {
            'slotKind': 'class',
            'slotLang': 'cpp',
            'sym': iface_cpp_sym,
        }
    ]

    # source
    emit_record({
        'loc': iface_loc,
        'source': 1,
        'syntax': 'idl',
        'pretty': f'IDL class {iface.name}',
        'sym': iface_idl_sym,
    })

    # target
    emit_record({
        'loc': iface_loc,
        'target': 1,
        'kind': 'idl',
        'pretty': iface.name,
        'sym': iface_idl_sym,
    })

    if iface.attributes.scriptable:
        iface_js_sym = f'#{ iface.name }'

        iface_slots.append({
            'slotKind': 'interface_name',
            'slotLang': 'js',
            'sym': iface_js_sym,
        })

    iface_supers = []

    if iface.base:
        (lineno, colno) = find_line_column(text, iface.base, iface.location._lexpos)
        base_idl_sym =f'XPIDL_{ iface.base }'

        base_loc = '%d:%d-%d' % (lineno, colno, colno + len(iface.base))

        iface_supers.append({
            'sym': base_idl_sym,
        })

        # Base source
        emit_record({
            'loc': base_loc,
            'source': 1,
            'syntax': 'idl',
            'pretty': f'IDL class {iface.base}',
            'sym': base_idl_sym,
        })

        # Base target
        emit_record({
            'loc': base_loc,
            'target': 1,
            'syntax': 'idl',
            'pretty': iface.base,
            'sym': base_idl_sym,
        })

    iface_methods = []
    iface_fields = []

    #print p.name
    #print 'BASE', p.base
    for m in iface.members:
        name = getattr(m, 'name', '')
        (lineno, colno) = find_line_column(text, name, m.location._lexpos)

        # Want to deal with attributes like 00, as well as ConstMember

        if isinstance(m, xpidl.Method):
            method_pretty = f'{iface.name}::{m.name}'
            method_cpp_sym = methods.get(cpp_method_name(m))
            method_idl_sym = f'XPIDL_{iface.name}_{m.name}'

            method_loc = '%d:%d-%d' % (lineno, colno, colno + len(m.name))

            # target
            emit_record({
                'loc': method_loc,
                'target': 1,
                'kind': 'idl',
                'pretty': method_pretty,
                'sym': method_idl_sym,
            })

            # Source
            emit_record({
                'loc': method_loc,
                'source': 1,
                'syntax': 'idl',
                'pretty': f'IDL method {method_pretty}',
                'sym': method_idl_sym,
            })

            method_slots = []
            if method_cpp_sym:
                method_slots.append({
                    'slotKind': 'method',
                    'slotLang': 'cpp',
                    'sym': method_cpp_sym,
                })


            if not m.noscript:
                method_js_sym = f'#{m.name}'
                method_slots.append({
                    'slotKind': 'method',
                    'slotLang': 'js',
                    'sym': method_js_sym,
                })

            # structured
            emit_record({
                'loc': method_loc,
                'structured': 1,
                'pretty': method_pretty,
                'sym': method_idl_sym,
                'kind': 'method',
                'implKind': 'idl',
                'bindingSlots': method_slots,
            })

            iface_methods.append({
                'pretty': method_pretty,
                'sym': method_idl_sym,
                'props': [],
            })

        elif isinstance(m, xpidl.Attribute):
            attr_pretty = f'{iface.name}::{m.name}'
            attr_idl_sym = f'XPIDL_{iface.name}_{m.name}'
            getter_cpp_sym = methods.get(cpp_getter_name(m))

            attr_loc = '%d:%d-%d' % (lineno, colno, colno + len(m.name))

            # target
            emit_record({
                'loc': attr_loc,
                'target': 1,
                'kind': 'idl',
                'pretty': attr_pretty,
                'sym': attr_idl_sym,
            })

            # source
            emit_record({
                'loc': attr_loc,
                'source': 1,
                'syntax': 'idl',
                'pretty': 'IDL attribute %s' % attr_pretty,
                'sym': attr_idl_sym,
            })

            attr_slots = []
            if getter_cpp_sym:
                attr_slots.append({
                    'slotKind': 'getter',
                    'slotLang': 'cpp',
                    'sym': getter_cpp_sym,
                })

            if not m.readonly:
                setter_cpp_sym = methods.get(cpp_setter_name(m))

                if setter_cpp_sym:
                    attr_slots.append({
                        'slotKind': 'setter',
                        'slotLang': 'cpp',
                        'sym': setter_cpp_sym,
                    })

            if not m.noscript:
                attr_js_sym = f'#{m.name}'
                attr_slots.append({
                    'slotKind': 'attribute',
                    'slotLang': 'js',
                    'sym': attr_js_sym,
                })

            emit_record({
                'loc': attr_loc,
                'structured': 1,
                'pretty': attr_pretty,
                'sym': attr_idl_sym,
                'kind': 'field',
                'implKind': 'idl',
                'bindingSlots': attr_slots,
            })

            iface_fields.append({
                'pretty': attr_pretty,
                'sym': attr_idl_sym,
                'props': [],
            })


        elif isinstance(m, xpidl.ConstMember):
            const_pretty = f'{iface.name}::{m.name}'
            const_idl_sym = f'XPIDL_{iface.name}_{m.name}'
            const_cpp_sym = find_enum(enums, m.name)

            const_loc = '%d:%d-%d' % (lineno, colno, colno + len(m.name))

            # target
            emit_record({
                'loc': const_loc,
                'target': 1,
                'kind': 'idl',
                'pretty': const_pretty,
                'sym': const_idl_sym,
            })

            # source
            emit_record({
                'loc': const_loc,
                'source': 1,
                'syntax': 'idl',
                'pretty': 'IDL constant %s' % const_pretty,
                'sym': const_idl_sym,
            })

            const_slots = [
                {
                    'slotKind': 'const',
                    'slotLang': 'cpp',
                    'sym': const_cpp_sym,
                }
            ]

            # there's no such thing as noscript for a const, so we just go based
            # on whether the interface is itself scriptable.
            # XXX uh, should we have been checking that above too?
            if iface.attributes.scriptable:
                const_js_sym = f'#{m.name}'
                const_slots.append({
                    'slotKind': 'const',
                    'slotLang': 'js',
                    'sym': const_js_sym,
                })

            emit_record({
                'loc': const_loc,
                'structured': 1,
                'pretty': const_pretty,
                'sym': const_idl_sym,
                'kind': 'enum',
                'implKind': 'idl',
                'bindingSlots': const_slots,
            })

            iface_fields.append({
                'pretty': const_pretty,
                'sym': const_idl_sym,
                'props': [],
            })


    emit_record({
        'loc': '%d:%d-%d' % (lineno, colno, colno + len(iface.name)),
        'structured': 1,
        'pretty': iface.name,
        'sym': iface_idl_sym,
        'kind': 'class',
        'implKind': 'idl',
        'bindingSlots': iface_slots,
        'supers': iface_supers,
        'methods': iface_methods,
        'fields': iface_fields
    })

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
    (methods, enums) = analysis
    p = xpidl.IDLParser()

    try:
        r = p.parse(text, filename=fname)
    except xpidl.IDLError as e:
        print('Syntax error in IDL', fname, file=sys.stderr)
        raise e
        sys.exit(1)
    print('XPIDL: Parsed', fname, 'into', len(r.productions), 'productions', file=sys.stderr)
    for p in r.productions:
        if isinstance(p, xpidl.Interface):
            handle_interface(methods.get(p.name, {}), enums, p)
else:
    print('XPIDL: No C++ analysis data found for', fname, file=sys.stderr)
