# Functions for working with the `nsresult`.

from __future__ import absolute_import
import json
import re
import sys
import os.path
from logger import log

repo_data = {}


def load(config):
    global repo_data

    for tree_name in config['trees']:
        log('Loading %s', tree_name)
        objdir = config['trees'][tree_name]['objdir_path']
        list_path = os.path.join(objdir, 'xpcom', 'base', 'ErrorList.h')

        if not os.path.exists(list_path):
            continue

        mod_offset = 0
        mods = {}
        codes = {}

        try:
            with open(list_path, 'r') as f:
                for line in f:
                    line = line.strip()

                    m = re.match(r'^\s*#\s*define\s+(NS_ERROR_MODULE_[A-Za-z0-9_]*)\s+([0-9]+)$', line)
                    if m:
                        mod = m.group(1)
                        n = int(m.group(2))
                        if mod == 'NS_ERROR_MODULE_BASE_OFFSET':
                            mod_offset = n
                        else:
                            mods[n] = mod
                        continue

                    m = re.match(r'^\s*(NS_[A-Za-z0-9_]*)\s*=\s*(0x[0-9A-Fa-f]+),?$', line)
                    if m:
                        name = m.group(1)
                        n = int(m.group(2), 16)
                        if n in codes:
                            codes[n].append(name)
                        else:
                            codes[n] = [name]
        except:
            pass

        repo_data[tree_name] = (mod_offset, mods, codes)


# From https://searchfox.org/firefox-main/source/xpcom/base/nsError.h
#
# #define NS_ERROR_SEVERITY_SUCCESS 0
# #define NS_ERROR_SEVERITY_ERROR 1
#
# #define NS_ERROR_GENERATE(sev, module, code)                            \
#   (nsresult)(((uint32_t)(sev) << 31) |                                  \
#              ((uint32_t)(module + NS_ERROR_MODULE_BASE_OFFSET) << 16) | \
#              ((uint32_t)(code)))

def decompose(n, mod_offset):
    if (n >> 31) & 1:
        sev = 'NS_ERROR_SEVERITY_ERROR'
    else:
        sev = 'NS_ERROR_SEVERITY_SUCCESS'
    mod = ((n >> 16) & 0x7fff) - mod_offset
    code = n & 0xffff

    return (sev, mod, code)


def lookup(tree_name, query):
    """Returns one of the following:
    if tree is not found:
      (query, None, None, None, None, None)
    if invalid code:
      (query, None, None, None, None, None)
    if variant is found:
      (raw_code, variant_names, None, None, None, None)
    if generated with known module:
      (raw_code, None, severity_name, module_name, None, raw_code)
    if generated with unknown module:
      (raw_code, None, severity_name, None, raw_module, None, raw_code)
    """
    if tree_name not in repo_data:
        return (query, None, None, None, None, None)

    (mod_offset, mods, codes) = repo_data[tree_name]

    hex_n = -1
    dec_n = -1

    try:
        hex_n = int(query, 16)
    except:
        pass

    try:
        dec_n = int(query, 10)
    except:
        pass

    if hex_n == -1 and dec_n == -1:
        return (query, None, None, None, None, None)

    if hex_n in codes:
        return (hex_n, codes[hex_n], None, None, None, None)
    if dec_n in codes:
        return (dec_n, codes[dec_n], None, None, None, None)

    if hex_n != -1:
        (hex_sev, hex_mod, hex_code) = decompose(hex_n, mod_offset)
        if hex_mod in mods:
            return (hex_n, None, hex_sev, mods[hex_mod], None, hex_code)

    if dec_n != -1:
        (dec_sev, dec_mod, dec_code) = decompose(dec_n, mod_offset)
        if dec_mod in mods:
            return (dec_n, None, dec_sev, mods[dec_mod], None, dec_code)

    if hex_n != -1:
        if hex_mod >= 0:
            return (hex_n, None, hex_sev, None, hex_mod, hex_code)

    if dec_n != -1:
        if dec_mod >= 0:
            return (dec_n, None, dec_sev, None, dec_mod, dec_code)

    return (query, None, None, None, None, None)


if __name__ == '__main__':
    load(json.load(open(sys.argv[1])))
    print(lookup(sys.argv[2], sys.argv[3]))
