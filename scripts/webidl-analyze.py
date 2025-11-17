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

        m = re.match(r'[LK]?([0-9]+)([a-zA-Z0-9_]+)', inner)
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


def capitalize(name):
    return name[0].upper() + name[1:]


class CppSymbolMemberItem:
    '''Represents single member's C++ symbols

    Single member can have multiple symbols for the following reasons:
      * method can have overloads
      * signature can be different between architecture

    If there are N overloads, there will be one binding C++ function
    (generated code) and N C++ impls (non-generated code), for each
    architecture.

    All overload impls are supposed to br called from single binding function.
    '''

    def __init__(self):
        # A list of C++ binding function symbols.
        self.binding_syms = []

        # A list of C++ implementation function symbols.
        self.impl_syms = []

    def add_binding(self, sym):
        '''Add a binding's C++ symbol.'''

        if sym in self.binding_syms:
            # Other architecture had the same symbol
            return

        self.binding_syms.append(sym)

    def add_impl(self, sym):
        '''Add a implementation's C++ symbol.'''

        if sym in self.impl_syms:
            # Other architecture had the same symbol
            return

        self.impl_syms.append(sym)

    def merge(self, other):
        self.binding_syms += other.binding_syms
        self.impl_syms += other.impl_syms


class CppSymbolsBuilder:
    '''Build the C++ symbol map'''

    def __init__(self, cpp_symbols):
        self.cpp_symbols = cpp_symbols

        # The interface name of the current C++ binding function.
        self.current_iface_name = None

        # The member name of the current C++ binding function.
        self.current_member_name = None

        # The CppSymbolMemberItem for the member for the
        # current C++ binding function.
        self.current_member_item = None

    @staticmethod
    def parse_sym(sym):
        '''Given C++ symbol, return the class/interface name and member name.
        The '_Binding' suffix is not removed.

        If the symbol doesn't match the binding or implementation's pattern,
        returns None for both.
        '''

        if not sym:
            return None, None

        if not sym.startswith('_Z'):
            return None, None

        idents = parse_mangled(sym)
        if not idents or len(idents) != 4:
            return None, None

        # All bindings and impls are directly inside mozilla::dom namespace,
        # with `_Binding` suffix added for bindings and no suffix for impls.
        if idents[0] != 'mozilla' or idents[1] != 'dom':
            return None, None

        return idents[2], idents[3]

    def maybe_add_binding(self, sym):
        '''If given symbol is C++ binding function, add it.'''

        iface_name, member_name = self.parse_sym(sym)
        if iface_name is None:
            return
        if not iface_name.endswith('_Binding'):
            return
        iface_name = iface_name.replace('_Binding', '')

        per_iface = self.cpp_symbols.setdefault(iface_name, {})
        if member_name in per_iface:
            member_item = per_iface[member_name]
        else:
            member_item = CppSymbolMemberItem()
            per_iface[member_name] = member_item

        self.current_iface_name = iface_name
        self.current_member_name = member_name
        self.current_member_item = member_item

        member_item.add_binding(sym)

    def maybe_add_impl(self, sym, contextsym):
        '''If given symbol is C++ implementation function for the
        current C++ binding function, add it.'''

        context_iface_name, context_member_name = self.parse_sym(contextsym)
        if context_iface_name is None:
            return
        if not context_iface_name.endswith('_Binding'):
            return
        context_iface_name = context_iface_name.replace('_Binding', '')

        if self.current_iface_name != context_iface_name:
            return
        if self.current_member_name != context_member_name:
            return

        iface_name, member_name = self.parse_sym(sym)

        if iface_name is None:
            return
        if iface_name != context_iface_name:
            return

        if context_member_name.startswith('get_'):
            name = capitalize(context_member_name[4:])
            expected = [name, "Get" + name]
        elif context_member_name.startswith('set_'):
            name = capitalize(context_member_name[4:])
            expected = ["Set" + name]
        else:
            name = capitalize(context_member_name)
            expected = [name]

        if member_name not in expected:
            return

        self.current_member_item.add_impl(sym)


def read_cpp_analysis_one(path, cpp_symbols):
    '''Read given analysis file and collect C++ symbols for generated code.'''

    if not os.path.exists(path):
        return

    try:
        lines = open(path).readlines()
    except IOError as e:
        return

    builder = CppSymbolsBuilder(cpp_symbols)

    for line in lines:
        try:
            j = json.loads(line.strip())
        except ValueError as e:
            print('Syntax error in JSON file', path, line.strip(), file=sys.stderr)
            raise e

        if 'target' not in j:
            continue

        kind = j['kind']

        if kind in ('decl', 'def'):
            builder.maybe_add_binding(j['sym'])

        elif kind == 'use':
            builder.maybe_add_impl(j['sym'], j.get('contextsym', ''))


def read_cpp_analysis(analysis_root, local_path, bindings_local_path):
    '''Read analysis files for given WebIDL file and collect C++ symbols
    for generated code.'''

    base = os.path.basename(local_path)
    (idl_name, suffix) = os.path.splitext(base)
    header_name = idl_name + 'Binding.h'
    cpp_name = idl_name + 'Binding.cpp'
    cpp_symbols = {}

    if bindings_local_path:
        # Override the paths for bindings for testing.
        p = os.path.join(analysis_root, bindings_local_path, 'include', header_name)
        read_cpp_analysis_one(p, cpp_symbols)
        p = os.path.join(analysis_root, bindings_local_path, 'src', cpp_name)
        read_cpp_analysis_one(p, cpp_symbols)
    else:
        generated_dir = os.path.join(analysis_root, '__GENERATED__')
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


def get_location_filename(location):
    '''Absorb the difference of the Location class across versions.
    See bug 1884321.'''

    filename = location.filename
    if type(filename) == str:
        # 125+ has plain string field.
        return filename

    return location.filename()


def get_records(target):
    '''Get the record list for given IDLObject's file.'''
    local_path = get_location_filename(target.location)
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


def append_slot(slots, kind, lang, impl_kind, sym):
    record = {
        'slotKind': kind,
        'slotLang': lang,
        'ownerLang': 'idl',
        'sym': sym,
    }
    if impl_kind is not None:
        record['implKind'] = impl_kind
    slots.append(record)


def handle_method(records, iface_name, methods, cpp_symbols, target):
    '''Emit analysis record for IDLMethod in interface or namespace.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = f'{iface_name}::{name}'
    idl_sym = f'WEBIDL_{iface_name}_{name}'
    cpp_item = cpp_symbols.get(cpp_method_name(target, name), None)
    js_sym = f'#{name}'
    is_constructor = name == 'constructor'

    emit_source(records, loc, 'idl', 'method', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    if cpp_item:
        for sym in cpp_item.binding_syms:
            append_slot(slots, 'method', 'cpp', 'binding', sym)
        for sym in cpp_item.impl_syms:
            append_slot(slots, 'method', 'cpp', 'impl', sym)
    append_slot(slots, 'method', 'js', None, js_sym)

    emit_structured(records, loc, 'method', pretty, idl_sym,
                    slots=slots)

    # Before resolve, overloads are represented as separate IDLMethod.
    assert len(target._overloads) == 1

    overload = target._overloads[0]
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
    getter_cpp_item = cpp_symbols.get(cpp_getter_name(target, name), None)
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'attribute', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    if getter_cpp_item:
        for sym in getter_cpp_item.binding_syms:
            append_slot(slots, 'getter', 'cpp', 'binding', sym)
        for sym in getter_cpp_item.impl_syms:
            append_slot(slots, 'getter', 'cpp', 'impl', sym)
    if not target.readonly:
        setter_cpp_item = cpp_symbols.get(cpp_setter_name(target, name), None)
        if setter_cpp_item:
            for sym in  setter_cpp_item.binding_syms:
                append_slot(slots, 'setter', 'cpp', 'binding', sym)
            for sym in  setter_cpp_item.impl_syms:
                append_slot(slots, 'setter', 'cpp', 'impl', sym)
    append_slot(slots, 'attribute', 'js', None, js_sym)

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
    cpp_item = cpp_symbols.get(name, None)
    js_sym = f'#{name}'

    emit_source(records, loc, 'idl', 'const', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    if cpp_item:
        for sym in cpp_item.binding_syms:
            append_slot(slots, 'const', 'cpp', None, sym)
    append_slot(slots, 'const', 'js', None, js_sym)

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
        cpp_sym = f'NS_mozilla::dom::{iface_name}Iterator_Binding'
    elif target.maplikeOrSetlikeOrIterableType == 'asynciterable':
        cpp_sym = f'NS_mozilla::dom::{iface_name}AsyncIterator_Binding'
    else:
        print(f'warning: WebIDL: Unknown maplikeOrSetlikeOrIterableType: {target.maplikeOrSetlikeOrIterableType}',
              file=sys.stderr)

    emit_source(records, loc, 'idl', 'method', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    append_slot(slots, 'class', 'cpp', None, cpp_sym)

    emit_structured(records, loc, 'method', pretty, idl_sym,
                    slots=slots)

    if target.keyType:
        handle_type(records, target.keyType)
    if target.valueType:
        handle_type(records, target.valueType)
    if hasattr(target, 'argList'):
        for arg in target.argList:
            handle_argument(records, arg)


def handle_interface_or_namespace(records, target, mixin_consumers_map=None):
    '''Emit analysis record for IDLInterface, IDLInterfaceMixin, IDLNamespace,
    or IDLPartialInterfaceOrNamespace.'''

    is_mixin = isinstance(target, WebIDL.IDLInterfaceMixin)

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
    pretty = name
    idl_sym = f'WEBIDL_{name}'
    if not is_mixin:
        cpp_sym = f'NS_mozilla::dom::{name}_Binding'
    js_sym = f'#{name}'

    if not is_mixin:
        local_path = get_location_filename(target.location)
        cpp_analysis = cpp_analysis_map.get(local_path, None)
        if cpp_analysis is None:
            print('warning: WebIDL: No C++ analysis data found for', local_path, file=sys.stderr)
            cpp_symbols = {}
        else:
            cpp_symbols = cpp_analysis.get(name, {})
    else:
        cpp_symbols = {}
        for iface in mixin_consumers_map.get(name, []):
            iface_name = iface.identifier.name
            local_path = get_location_filename(iface.location)
            cpp_analysis = cpp_analysis_map.get(local_path, None)
            if cpp_analysis is None:
                print('warning: WebIDL: No C++ analysis data found for', local_path, file=sys.stderr)
            else:
                iface_cpp_symbols = cpp_analysis.get(iface_name, {})
                for prop, item in iface_cpp_symbols.items():
                    if prop not in cpp_symbols:
                        cpp_symbols[prop] = CppSymbolMemberItem()

                    cpp_symbols[prop].merge(item)

    emit_source(records, loc, 'idl', 'class', pretty, idl_sym)
    emit_target(records, loc, 'idl', pretty, idl_sym)

    slots = []
    if not is_mixin:
        append_slot(slots, 'class', 'cpp', None, cpp_sym)
        append_slot(slots, 'interface_name', 'js', None, js_sym)

    if not is_mixin:
        supers = []
        if hasattr(target, 'parent') and target.parent:
            handle_super(records, supers, target.parent)
    else:
        supers = None

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
            print(f'warning: WebIDL: Unknown member production: {member.__class__.__name__}',
                  file=sys.stderr)

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
    append_slot(slots, 'attribute', 'cpp', None, cpp_sym)
    append_slot(slots, 'attribute', 'js', None, js_sym)

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
    append_slot(slots, 'class', 'cpp', None, cpp_sym)

    supers = []
    if hasattr(target, 'parent') and target.parent:
        handle_super(records, supers, target.parent)

    fields = []
    for member in target.members:
        if isinstance(member, WebIDL.IDLArgument):
            handle_dictionary_field(records, name, cpp_sym,
                                    fields, member)
        else:
            print(f'warning: WebIDL: Unknown member production: {member.__class__.__name__}',
                  file=sys.stderr)

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
    append_slot(slots, 'const', 'cpp', None, cpp_sym)

    emit_structured(records, loc, 'const', pretty, idl_sym,
                    slots=slots)


def handle_typedef(records, target):
    '''Emit analysis record for IDLTypedef.'''

    name = target.identifier.name
    loc = to_loc(target.identifier.location, name)
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


def parse_files(index_root, files_root, analysis_root, cache_dir, bindings_local_path):
    '''Parse all WebIDL files and load corresponding C++ analysis files.'''

    parser = WebIDL.Parser(cache_dir)

    for local_path in sys.stdin:
        local_path = local_path.strip()

        if local_path.startswith('__GENERATED__/'):
            fname = os.path.join(index_root, 'objdir', local_path.replace('__GENERATED__/', ''))
        else:
            fname = os.path.join(files_root, local_path)

        if not os.path.exists(fname):
            # New filename from 144 (See bug 1919582).
            if 'CSSStyleProperties.webidl' in fname:
                continue
            # Old filename.
            if 'CSS2Properties.webidl' in fname:
                continue

        lines = preprocess(open(fname).readlines())
        text = ''.join(lines)
        cpp_analysis_map[local_path] = read_cpp_analysis(analysis_root, local_path, bindings_local_path)

        try:
            parser.parse(text, local_path)
        except WebIDL.WebIDLError as e:
            print('WebIDL: Syntax error in IDL', fname, file=sys.stderr)
            raise e

    # NOTE: Do not call parser.finish() here because we need raw identifiers and
    #       raw productions, and we don't need auto-generated items.
    return parser._productions


def collect_mixin_consumers_map(productions):
    iface_map = {}
    for target in productions:
        if isinstance(target, WebIDL.IDLInterface):
            iface_name = target.identifier.name
            iface_map[iface_name] = target

    mixin_consumers_map = {}
    for target in productions:
        if isinstance(target, WebIDL.IDLIncludesStatement):
            iface_name = target.interface.identifier.name
            mixin = target.mixin.identifier.name

            if mixin not in mixin_consumers_map:
                mixin_consumers_map[mixin] = []

            if iface_name in iface_map:
                iface = iface_map.get(iface_name)
                mixin_consumers_map[mixin].append(iface)

    return mixin_consumers_map


def handle_productions(productions):
    '''Emit analysis records for all productions.'''

    mixin_consumers_map = collect_mixin_consumers_map(productions)

    for target in productions:
        if isinstance(target, WebIDL.IDLInterfaceOrNamespace):
            records = get_records(target)
            handle_interface_or_namespace(records, target)
        elif isinstance(target, WebIDL.IDLPartialInterfaceOrNamespace):
            records = get_records(target)
            handle_interface_or_namespace(records, target)
        elif isinstance(target, WebIDL.IDLInterfaceMixin):
            records = get_records(target)
            handle_interface_or_namespace(records, target, mixin_consumers_map)
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
            print(f'warning: WebIDL: Unknown top-level production: {target.__class__.__name__}',
                  file=sys.stderr)


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
bindings_local_path = sys.argv[5]
if bindings_local_path == 'null':
    bindings_local_path = None

productions = parse_files(index_root, files_root, analysis_root, cache_dir, bindings_local_path)
handle_productions(productions)
write_files(analysis_root)
