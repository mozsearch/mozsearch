#!/usr/bin/env python3

# See the comment at the top of idl-analyze.py file for the details about
# how IDL indexing works.

import json
import os.path
import re
import sys

import WebIDL

# local path => records.
analysis_map = {}

# local path => interface name => member name => member symbols.
cpp_analysis_map = {}


def parse_mangled(mangled):
    def parse_inner(idents, inner):
        if inner[0] == 'E':
            return True

        m = re.match(r'L?([0-9]+)([a-zA-Z0-9_]+)', inner)
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


def read_cpp_analysis_one(path, cpp_symbols):
    '''Read given analysis file and collect C++ symbols for generated code.'''

    if not os.path.exists(path):
        return

    try:
        lines = open(path).readlines()
    except IOError as e:
        return

    for line in lines:
        try:
            j = json.loads(line.strip())
        except ValueError as e:
            print('Syntax error in JSON file', path, line.strip(), file=sys.stderr)
            raise e

        if 'target' in j and j['kind'] in ('decl', 'def'):
            if not j['sym'].startswith('_Z'):
                continue

            idents = parse_mangled(j['sym'])
            if not idents or len(idents) != 4:
                continue

            assert idents[0] == 'mozilla'
            assert idents[1] == 'dom'
            binding_name = idents[2].replace('_Binding', '')
            member_name = idents[3]

            cpp_symbols.setdefault(binding_name, {}).setdefault(member_name, []).append(j['sym'])


def read_cpp_analysis(index_root, local_path):
    '''Read analysis files for given WebIDL file and collect C++ symbols
    for generated code.'''

    base = os.path.basename(local_path)
    (idl_name, suffix) = os.path.splitext(base)
    header_name = idl_name + 'Binding.h'
    cpp_name = idl_name + 'Binding.cpp'
    cpp_symbols = {}

    generated_dir = os.path.join(index_root, 'analysis', '__GENERATED__')
    header_local_path = os.path.join('dist', 'include', 'mozilla', 'dom', header_name)
    cpp_local_path = os.path.join('dom', 'bindings', cpp_name)

    p = os.path.join(generated_dir, header_local_path)
    read_cpp_analysis_one(p, cpp_symbols)
    p = os.path.join(generated_dir, cpp_local_path)
    read_cpp_analysis_one(p, cpp_symbols)

    for name in os.listdir(generated_dir):
        if name.startswith('__'):
            p = os.path.join(generated_dir, name, header_local_path)
            read_cpp_analysis_one(p, cpp_symbols)
            p = os.path.join(generated_dir, name, cpp_local_path)
            read_cpp_analysis_one(p, cpp_symbols)

    return cpp_symbols


def get_binary_name(target):
    binary_name = target.getExtendedAttribute('BinaryName')
    if not binary_name:
        return None
    return binary_name[0]


def cpp_method_name(method, name):
    '''Return the C++ pretty name for this method.'''
    if isinstance(method, WebIDL.IDLConstructor):
        return '_constructor'

    binary_name = get_binary_name(method)
    if binary_name:
        return binary_name
    return name


def cpp_getter_name(attr, name):
    '''Return the C++ pretty name for this getter.'''
    binary_name = get_binary_name(attr)
    if binary_name:
        return 'get_' + binary_name
    return 'get_' + name


def cpp_setter_name(attr, name):
    '''Return the C++ pretty name for this setter.'''
    binary_name = get_binary_name(attr)
    if binary_name:
        return 'set_' + binary_name
    return 'set_' + name


def IDLToCIdentifier(name):
    return name.replace('-', '_')


def cpp_dictionary_field_name(name):
    '''Return the C++ pretty name for this dictionary field.'''
    return 'm' + name[0].upper() + IDLToCIdentifier(name[1:])


def to_loc_with(lineno, colno, name_len):
    return f'{lineno}:{colno}-{colno + name_len}'


def to_loc(location, name):
    location.resolve()
    return to_loc_with(location._lineno, location._colno, len(name))


def get_records(target):
    '''Get the record list for given IDLObject's file.'''
    local_path = target.location.filename
    if local_path in analysis_map:
        return analysis_map[local_path]

    records = []
    analysis_map[local_path] = records
    return records


def emit_source(records, loc, syntax, pretty_prefix, pretty, sym):
    '''Emit AnalysisSource record.'''
    records.append({
        'loc': loc,
        'source': 1,
        'syntax': syntax,
        'pretty': f'IDL {pretty_prefix} {pretty}',
        'sym': sym,
    })


def emit_target(records, loc, kind, pretty, sym):
    '''Emit AnalysisTarget record.'''
    records.append({
        'loc': loc,
        'target': 1,
        'kind': kind,
        'pretty': pretty,
        'sym': sym,
    })


def emit_structured(records, loc, kind, pretty, sym,
                    slots=None, supers=None,
                    methods=None, fields=None):
    '''Emit AnalysisStructured record.'''
    record = {
        'loc': loc,
        'structured': 1,
        'pretty': pretty,
        'sym': sym,
        'kind': kind,
        'implKind': 'idl',
    }
    if slots:
        record['bindingSlots'] = slots
    if supers:
        record['supers'] = supers
    if methods:
        record['methods'] = methods
    if fields:
        record['fields']= fields

    records.append(record)


def handle_simple_type(records, target):
    '''Emit analysis record for simple IDLType subclasses.'''

    if isinstance(target.name, str):
        name = target.name
    else:
        name = target.name.name
    loc = to_loc(target.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'type', 'type', pretty, idl_sym)
    emit_target(records, loc, 'use', pretty, idl_sym)


def handle_type(records, target):
    '''Emit analysis record for IDLType subclasses.'''

    if isinstance(target, WebIDL.IDLNullableType):
        handle_type(records, target.inner)
        return
    if isinstance(target, WebIDL.IDLSequenceType):
        handle_type(records, target.inner)
        return
    if isinstance(target, WebIDL.IDLRecordType):
        handle_type(records, target.keyType)
        handle_type(records, target.inner)
        return
    if isinstance(target, WebIDL.IDLObservableArrayType):
        handle_type(records, target.inner)
        return
    if isinstance(target, WebIDL.IDLPromiseType):
        handle_type(records, target.inner)
        return
    if isinstance(target, WebIDL.IDLUnionType):
        for m in target.memberTypes:
            handle_type(records, m)
        return
    if isinstance(target, WebIDL.IDLBuiltinType):
        return

    assert isinstance(target, WebIDL.IDLUnresolvedType) or \
        isinstance(target, WebIDL.IDLTypedefType) or \
        isinstance(target, WebIDL.IDLCallbackType) or \
        isinstance(target, WebIDL.IDLWrapperType)

    handle_simple_type(records, target)


def handle_argument(records, target):
    '''Emit analysis record for IDLArgument in methods.'''

    handle_type(records, target.type)


def append_slot(slots, kind, lang, sym):
    slots.append({
        'slotKind': kind,
        'slotLang': lang,
        'ownerLang': 'idl',
        'sym': sym,

    })


def handle_method(records, iface_name, methods, cpp_symbols, target):
    '''Emit analysis record for IDLMethod in interface or namespace.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = f'{iface_name}::{name}'
    idl_sym = f'WEBIDL_{iface_name}_{name}'
    cpp_syms = cpp_symbols.get(cpp_method_name(target, name), [])
    js_sym = f'#{name}'
    is_constructor = name == 'constructor'

    emit_source(records, loc, 'idl', 'method', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    for sym in cpp_syms:
        append_slot(slots, 'method', 'cpp', sym)
    append_slot(slots, 'method', 'js', js_sym)

    emit_structured(records, loc, 'method', pretty, idl_sym,
                    slots=slots)

    for overload in target._overloads:
        if not is_constructor:
            handle_type(records, overload.returnType)
        for arg in overload.arguments:
            handle_argument(records, arg)

    methods.append({
        'pretty': pretty,
        'sym': idl_sym,
        'props': [],
    })


def handle_attribute(records, iface_name, fields, cpp_symbols, target):
    '''Emit analysis record for IDLAttribute in interface or namespace.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = f'{iface_name}::{name}'
    idl_sym = f'WEBIDL_{iface_name}_{name}'
    getter_cpp_syms = cpp_symbols.get(cpp_getter_name(target, name), [])
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'attribute', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    for sym in getter_cpp_syms:
        append_slot(slots, 'getter', 'cpp', sym)
    if not target.readonly:
        setter_cpp_syms = cpp_symbols.get(cpp_setter_name(target, name), [])
        for sym in  setter_cpp_syms:
            append_slot(slots, 'setter', 'cpp', sym)
    append_slot(slots, 'attribute', 'js', js_sym)

    emit_structured(records, loc, 'field', pretty, idl_sym,
                    slots=slots)

    handle_type(records, target.type)

    fields.append({
        'pretty': pretty,
        'sym': idl_sym,
        'props': [],
    })


def handle_const(records, iface_name, fields, cpp_symbols, target):
    '''Emit analysis record for IDLConst in interface or namespace.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = f'{iface_name}::{name}'
    idl_sym = f'WEBIDL_{iface_name}_{name}'
    cpp_syms = cpp_symbols.get(name, [])
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'const', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    for sym in cpp_syms:
        append_slot(slots, 'const', 'cpp', sym)
    append_slot(slots, 'const', 'js', js_sym)

    emit_structured(records, loc, 'field', pretty, idl_sym,
                    slots=slots)

    handle_type(records, target.type)

    fields.append({
        'pretty': pretty,
        'sym': idl_sym,
        'props': [],
    })


def handle_super(records, supers, target):
    '''Emit analysis record for references in super interface
    or super dictionary.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'type', 'class', pretty, idl_sym)
    emit_target(records, loc, 'use', pretty, idl_sym)

    supers.append({
        'sym': idl_sym,
    })


def handle_maplike_or_setlike_or_iterable(records, iface_name, target):
    '''Emit analysis record for IDLMaplikeOrSetlike, IDLIterable,
    or IDLAsyncIterable in interface.'''

    name = target.identifier.name.replace('__', '')
    loc = to_loc(target.identifier.location, name)
    pretty = f'{iface_name}::{target.maplikeOrSetlikeOrIterableType}'
    idl_sym = f'WEBIDL_{iface_name}_{target.maplikeOrSetlikeOrIterableType}'

    if target.maplikeOrSetlikeOrIterableType == 'maplike':
        cpp_sym = f'NS_mozilla::dom::{iface_name}_Binding::MaplikeHelpers'
    elif target.maplikeOrSetlikeOrIterableType == 'setlike':
        cpp_sym = f'NS_mozilla::dom::{iface_name}_Binding::SetlikeHelpers'
    elif target.maplikeOrSetlikeOrIterableType == 'iterable':
        # NOTE: loc is wrong. it points '<' after 'iterable'.
        # TODO: Fix the WebIDL parser.
        location = target.identifier.location
        loc = to_loc_with(location._lineno, location._colno - len(name), len(name))
        cpp_sym = f'NS_mozilla::dom::{iface_name}Iterator_Binding'
    elif target.maplikeOrSetlikeOrIterableType == 'asynciterable':
        cpp_sym = f'NS_mozilla::dom::{iface_name}AsyncIterator_Binding'
    else:
        print(f'WebIDL: Unknown maplikeOrSetlikeOrIterableType: {target.maplikeOrSetlikeOrIterableType}',
              file=sys.stderr)
        sys.exit(1)

    emit_source(records, loc, 'idl', 'method', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    append_slot(slots, 'class', 'cpp', cpp_sym)

    emit_structured(records, loc, 'method', pretty, idl_sym,
                    slots=slots)

    if target.keyType:
        handle_type(records, target.keyType)
    if target.valueType:
        handle_type(records, target.valueType)
    if hasattr(target, 'argList'):
        for arg in target.argList:
            handle_argument(records, arg)


def handle_interface_or_namespace(records, target):
    '''Emit analysis record for IDLInterface, IDLInterfaceMixin, IDLNamespace,
    or IDLPartialInterfaceOrNamespace.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'
    cpp_sym = f'NS_mozilla::dom::{name}_Binding'
    js_sym = f'#{name}'

    local_path = target.location.filename
    cpp_analysis = cpp_analysis_map.get(local_path, None)
    if cpp_analysis is None:
        print('warning: WebIDL: No C++ analysis data found for', local_path, file=sys.stderr)
        cpp_symbols = {}
    else:
        cpp_symbols = cpp_analysis.get(name, {})

    emit_source(records, loc, 'idl', 'class', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    append_slot(slots, 'class', 'cpp', cpp_sym)
    append_slot(slots, 'interface_name', 'js', js_sym)

    supers = []
    if hasattr(target, 'parent') and target.parent:
        handle_super(records, supers, target.parent)

    methods = []
    fields = []
    for member in target.members:
        if isinstance(member.identifier.location, WebIDL.BuiltinLocation):
            continue

        if isinstance(member, WebIDL.IDLMethod):
            handle_method(records, name, methods, cpp_symbols, member)
        elif isinstance(member, WebIDL.IDLAttribute):
            handle_attribute(records, name, methods, cpp_symbols, member)
        elif isinstance(member, WebIDL.IDLConst):
            handle_const(records, name, methods, cpp_symbols, member)
        elif isinstance(member, WebIDL.IDLMaplikeOrSetlike):
            handle_maplike_or_setlike_or_iterable(records, name, member)
        elif isinstance(member, WebIDL.IDLIterable):
            handle_maplike_or_setlike_or_iterable(records, name, member)
        elif isinstance(member, WebIDL.IDLAsyncIterable):
            handle_maplike_or_setlike_or_iterable(records, name, member)
        else:
            print(f'WebIDL: Unknown member production: {member.__class__.__name__}',
                  file=sys.stderr)
            sys.exit(1)

    emit_structured(records, loc, 'class', pretty, idl_sym,
                    slots=slots, supers=supers,
                    methods=methods, fields=fields)


def handle_dictionary_field(records, dictionary_name, dictionary_cpp_sym,
                            fields, target):
    '''Emit analysis record for IDLArgument in dictionary.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = f'{dictionary_name}.{name}'
    idl_sym = f'WEBIDL_{dictionary_name}_{name}'
    cpp_sym = f'F_<{dictionary_cpp_sym}>_{cpp_dictionary_field_name(name)}'
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'field', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    append_slot(slots, 'attribute', 'cpp', cpp_sym)
    append_slot(slots, 'attribute', 'js', js_sym)

    emit_structured(records, loc, 'field', pretty, idl_sym,
                    slots=slots)

    handle_type(records, target.type)

    fields.append({
        'pretty': pretty,
        'sym': idl_sym,
        'props': [],
    })


def handle_dictionary(records, target):
    '''Emit analysis record for IDLDictionary or IDLPartialDictionary.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'
    cpp_sym = f'T_mozilla::dom::{name}'
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'dictionary', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    append_slot(slots, 'class', 'cpp', cpp_sym)
    append_slot(slots, 'class', 'js', js_sym)

    supers = []
    if hasattr(target, 'parent') and target.parent:
        handle_super(records, supers, target.parent)

    fields = []
    for member in target.members:
        if isinstance(member, WebIDL.IDLArgument):
            handle_dictionary_field(records, name, cpp_sym,
                                    fields, member)
        else:
            print(f'WebIDL: Unknown member production: {member.__class__.__name__}',
                  file=sys.stderr)
            sys.exit(1)

    emit_structured(records, loc, 'class', pretty, idl_sym,
                    slots=slots, supers=supers,
                    fields=fields)


def handle_enum(records, target):
    '''Emit analysis record for IDLEnum.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'
    cpp_sym = f'T_mozilla::dom::{name}'
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'enum', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    append_slot(slots, 'const', 'cpp', cpp_sym)
    append_slot(slots, 'const', 'js', js_sym)

    emit_structured(records, loc, 'const', pretty, idl_sym,
                    slots=slots)


def get_typedef_loc(target, name):
    '''Get the location string for the typedef identifier.

    The IDLTypedef's identifier points the `typedef` token,
    and we need to find the location from the line.

    TODO: Fix the WebIDL parser.
    '''

    location = target.identifier.location
    location.resolve()

    line = location._line
    index = line.find(f' {name};')
    if index == -1:
        return to_loc(target.identifier.location, name)

    colno = index + 1

    return to_loc_with(location._lineno, colno, len(name))


def handle_typedef(records, target):
    '''Emit analysis record for IDLTypedef.'''

    name = target.identifier.name
    loc = get_typedef_loc(target, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'idl', 'type', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    handle_type(records, target.innerType)


def handle_callback(records, target):
    '''Emit analysis record for IDLCallback.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'idl', 'callback', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    handle_type(records, target._returnType)
    for arg in target._arguments:
        handle_argument(records, arg)


def handle_includes(records, target):
    '''Emit analysis record for IDLIncludesStatement.'''

    name = target.interface.identifier.name
    loc = to_loc(target.interface.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'type', 'class', pretty, idl_sym)
    emit_target(records, loc, 'use', pretty, idl_sym)

    name = target.mixin.identifier.name
    loc = to_loc(target.mixin.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'type', 'class', pretty, idl_sym)
    emit_target(records, loc, 'use', pretty, idl_sym)


def handle_external_interface(records, target):
    '''Emit analysis record for IDLExternalInterface.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'

    emit_source(records, loc, 'type', 'class', pretty, idl_sym)
    emit_target(records, loc, 'use', pretty, idl_sym)


def preprocess(lines):
    '''Remove macros from the input.

    This expects the macro doesn't have conflicting then-clause vs else-clause.'''
    result = []
    for line in lines:
        if line.startswith('#'):
            result.append('\n')
        else:
            result.append(line)
    return result


def parse_files(index_root, files_root, cache_dir):
    '''Parse all WebIDL files and load corresponding C++ analysis files.'''

    parser = WebIDL.Parser(cache_dir)

    for local_path in sys.stdin:
        local_path = local_path.strip()

        if local_path.startswith('__GENERATED__/'):
            fname = os.path.join(index_root, 'objdir', local_path.replace('__GENERATED__/', ''))
        else:
            fname = os.path.join(files_root, local_path)

        lines = preprocess(open(fname).readlines())
        text = ''.join(lines)
        cpp_analysis_map[local_path] = read_cpp_analysis(index_root, local_path)

        try:
            parser.parse(text, local_path)
        except WebIDL.WebIDLError as e:
            print('WebIDL: Syntax error in IDL', fname, file=sys.stderr)
            raise e

    # NOTE: Do not call parser.finish() here because we need raw identifiers and
    #       raw productions, and we don't need auto-generated items.
    return parser._productions


def handle_productions(productions):
    '''Emit analysis records for all productions.'''

    for target in productions:
        if isinstance(target, WebIDL.IDLInterfaceOrInterfaceMixinOrNamespace):
            records = get_records(target)
            handle_interface_or_namespace(records, target)
        elif isinstance(target, WebIDL.IDLPartialInterfaceOrNamespace):
            records = get_records(target)
            handle_interface_or_namespace(records, target)
        elif isinstance(target, WebIDL.IDLDictionary):
            records = get_records(target)
            handle_dictionary(records, target)
        elif isinstance(target, WebIDL.IDLPartialDictionary):
            records = get_records(target)
            handle_dictionary(records, target)
        elif isinstance(target, WebIDL.IDLEnum):
            records = get_records(target)
            handle_enum(records, target)
        elif isinstance(target, WebIDL.IDLTypedef):
            records = get_records(target)
            handle_typedef(records, target)
        elif isinstance(target, WebIDL.IDLCallback):
            records = get_records(target)
            handle_callback(records, target)
        elif isinstance(target, WebIDL.IDLIncludesStatement):
            records = get_records(target)
            handle_includes(records, target)
        elif isinstance(target, WebIDL.IDLExternalInterface):
            records = get_records(target)
            handle_external_interface(records, target);
        else:
            print(f'WebIDL: Unknown top-level production: {target.__class__.__name__}',
                  file=sys.stderr)
            sys.exit(1)


def write_files(analysis_root):
    '''Write analysis records for each file.'''

    for local_path, records in analysis_map.items():
        analysis_path = os.path.join(analysis_root, local_path)
        print('WebIDL: Generating', analysis_path, file=sys.stderr)

        parent = os.path.dirname(analysis_path)
        os.makedirs(parent, exist_ok=True)

        with open(analysis_path, 'w') as fh:
            for r in records:
                print(json.dumps(r), file=fh)


index_root = sys.argv[1]
files_root = sys.argv[2]
analysis_root = sys.argv[3]
cache_dir = sys.argv[4]

productions = parse_files(index_root, files_root, cache_dir)
handle_productions(productions)
write_files(analysis_root)
