#!/usr/bin/env python3

import json
import os
import re
import sys


# imported from m-c/modules/libpref/init/generate_static_pref_list.py
def mk_id(name):
    "Replace '.' and '-' with '_', e.g. 'foo.bar-baz' becomes 'foo_bar_baz'."
    return name.replace('.', '_').replace('-', '_')


def read_cpp_analysis_one(path, cpp_symbols):
    '''Read given analysis file and collect C++ symbols for generated code.'''

    if not os.path.exists(path):
        print('no', path)
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

        if 'target' not in j:
            continue

        if j['kind'] != 'def':
            continue

        sym = j['sym']
        m = re.match('.+StaticPrefs[0-9]+(.+)Ev$', sym)
        if m:
            id = m.group(1)
        else:
            m = re.match('.+JS5Prefs[0-9]+(.+)Ev$', sym)
            if m:
                id = f'javascript_options_{m.group(1)}'
            else:
                continue

        if id not in cpp_symbols:
            cpp_symbols[id] = set()
        cpp_symbols[id].add(sym)


def read_cpp_analysis(ns, yaml_path, bindings_local_path, analysis_root):
    '''Read analysis files for given StaticPrefs file and collect C++ symbols
    for generated code.'''

    header_name = f'StaticPrefList_{ns}.h'
    cpp_symbols = {}

    if bindings_local_path:
        # Override the paths for bindings for testing.
        p = os.path.join(analysis_root, bindings_local_path, header_name)
        read_cpp_analysis_one(p, cpp_symbols)
    else:
        generated_dir = os.path.join(analysis_root, '__GENERATED__')
        header_local_path = os.path.join(os.path.dirname(yaml_path), header_name)

        p = os.path.join(generated_dir, header_local_path)
        read_cpp_analysis_one(p, cpp_symbols)

        for name in os.listdir(generated_dir):
            if name.startswith('__'):
                p = os.path.join(generated_dir, name, header_local_path)
                read_cpp_analysis_one(p, cpp_symbols)

        if ns == 'javascript':
            # Load SpiderMonkey-specific binding.
            header_local_path = 'js/public/PrefsGenerated.h'

            p = os.path.join(generated_dir, header_local_path)
            read_cpp_analysis_one(p, cpp_symbols)

            for name in os.listdir(generated_dir):
                if name.startswith('__'):
                    p = os.path.join(generated_dir, name, header_local_path)
                    read_cpp_analysis_one(p, cpp_symbols)

    return cpp_symbols


pref_bindings_map = {}


def get_cpp_symbols(name, yaml_path, bindings_local_path, analysis_root):
    '''Get the corresponding binding C++ symbols for the preference name.
    Returns an empty list if no binding is found.'''
    ns = name.split('.')[0]

    if ns in pref_bindings_map:
        cpp_symbols = pref_bindings_map[ns]
    else:
        cpp_symbols = read_cpp_analysis(ns, yaml_path, bindings_local_path, analysis_root)
        pref_bindings_map[ns] = cpp_symbols

    id = mk_id(name)

    if id not in cpp_symbols:
        return []

    return cpp_symbols[id]


def to_loc_with(lineno, colno, name_len):
    return f'{lineno}:{colno}-{colno + name_len}'


def process_file(yaml_path, bindings_local_path, files_root, analysis_root):
    records = []

    with open(os.path.join(files_root, yaml_path), 'r') as f:
        lineno = 1
        for line in f:
            m = re.match('^(- name: )(.+)', line.rstrip())
            if m:
                prefix = m.group(1)
                name = m.group(2)

                loc = to_loc_with(lineno, len(prefix), len(name))
                pretty = f'StaticPrefs {name}'
                sym = f'PREFS_{mk_id(name)}'

                slots = []

                cpp_syms = get_cpp_symbols(name, yaml_path, bindings_local_path, analysis_root)
                for cpp_sym in cpp_syms:
                    slots.append({
                        'slotKind': 'getter',
                        'slotLang': 'cpp',
                        'ownerLang': 'prefs',
                        'sym': cpp_sym,
                    })

                records.append({
                    'loc': loc,
                    'source': 1,
                    'syntax': 'def',
                    'pretty': pretty,
                    'sym': sym,
                })

                records.append({
                    'loc': loc,
                    'target': 1,
                    'kind': 'def',
                    'pretty': pretty,
                    'sym': sym,
                })

                records.append({
                    'loc': loc,
                    'structured': 1,
                    'pretty': pretty,
                    'sym': sym,
                    'kind': 'prefs',
                    'implKind': 'StaticPrefs',
                    'bindingSlots': slots,
                })

            lineno += 1

    analysis_path = os.path.join(analysis_root, yaml_path)
    print('StaticPrefs: Generating', analysis_path, file=sys.stderr)

    parent = os.path.dirname(analysis_path)
    os.makedirs(parent, exist_ok=True)

    with open(analysis_path, 'w') as f:
        for r in records:
            print(json.dumps(r), file=f)


files_path = sys.argv[1]

bindings_local_path = sys.argv[2]
if bindings_local_path == 'null':
    bindings_local_path = ''

files_root = sys.argv[3]
analysis_root = sys.argv[4]

with open(files_path, 'r') as f:
    for line in f:
        process_file(line.strip(), bindings_local_path,
                     files_root, analysis_root)
