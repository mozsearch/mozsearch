#!/usr/bin/env python3

from __future__ import absolute_import
import six.moves.SimpleHTTPServer
from six.moves.BaseHTTPServer import HTTPServer
from six.moves.socketserver import ForkingMixIn
import six.moves.urllib.request, six.moves.urllib.parse, six.moves.urllib.error
import six.moves.urllib.parse
import sys
import os
import os.path
import json
import re
import subprocess
import signal
import time
import errno
import traceback
import collections

import crossrefs
import identifiers
import codesearch
from logger import log
from six.moves import range
from raw_search import RawSearchResults

def index_path(tree_name):
    return config['trees'][tree_name]['index_path']

# Simple globbing implementation, except ^ and $ are also allowed.
def parse_path_filter(filter):
    filter = filter.replace('(', '\\(')
    filter = filter.replace(')', '\\)')
    filter = filter.replace('|', '\\|')
    filter = filter.replace('.', '\\.')

    def star_repl(m):
        if m.group(0) == '*':
            return '[^/]*'
        else:
            return '.*'
    filter = re.sub(r'\*\*|\*', star_repl, filter)

    filter = filter.replace('?', '.')

    def repl(m):
        s = m.group(1)
        components = s.split(',')
        s = '|'.join(components)
        return '(' + s + ')'
    filter = re.sub('{([^}]*)}', repl, filter)

    return filter

key_remapping = { 'uses': 'Uses', 'defs': 'Definitions', 'assignments': 'Assignments',
                  'decls': 'Declarations', 'idl': 'IDL', 'callees': None }

def merge_defs_from_symbols_as(tree_name, mix_target, symbol_names, as_key):
    '''
    Helper for `expand_keys` to build an aggregate path hit list to be stored
    as `as_key` in the `mix_target` consisting of the definitions for each
    provided symbol name, augmented with some kind of hacky hint for the UI so
    it can know to generate a search link for each specific type.
    '''
    # Do not do anything if there's too many results!
    if len(symbol_names) >= 50:
        return

    aggr_defs = []
    for symbol_name in symbol_names:
        info = crossrefs.lookup_single_symbol(tree_name, symbol_name)
        if info is None or 'defs' not in info:
            continue

        defs = info['defs']
        for path_hit in defs:
            path_hit['lines'][0]['upsearch'] = 'symbol:' + symbol_name
            aggr_defs.append(path_hit)

    if len(aggr_defs):
        mix_target[as_key] = aggr_defs


def expand_keys(tree_name, new_keyed, traverse_relations=True, depth=0):
    '''
    Converts to the old Uses/Definitions/Assignments/Declarations/IDL rep
    from the new uses/defs/assignments/decls/idl rep, dropping 'callees'
    entries.  Performs the mutation in-place which also means keys that aren't
    re-mapped are passed through untouched.

    ## New relation-traversing support!

    To help address the regression in the handling of overridden methods, we
    now will also investigate the "meta" field and induce synthetic keys
    ["Overrides", "Overridden By", "Superclasses", "Subclasses"] if
    `traverse_relations` is set to True.

    Our general UX goal (operating within the existing "search-not-sorch" data
    model) is:
    - If showing a method which has overrides:
      - We will show an "Overridden By" section whose hits will be the
        definitions of the overrides and exposes a "(search using this symbol)"
        upsell.
    - If showing a method which is itself an override of something else:
      - We will show an "Overrides" section whose hits will be the definitions
        of the thing we are overriding and upsells "(search using this symbol)".
    - We do the same thing as the above for "Superclasses" and "Subclasses".
    - If there will be more than 50 results, we don't attempt to show anything
      out of concern for overwhelming the server.
    '''
    for new_name, old_name in key_remapping.items():
        if new_name in new_keyed:
            # just drop records that the old names don't know how to handle.
            if old_name is None:
                new_keyed.pop(new_name)
            else:
                new_keyed[old_name] = new_keyed.pop(new_name)

    if 'meta' in new_keyed:
        if traverse_relations:
            # lookup_merging will have wrapped the value into a list
            meta_arr = new_keyed.pop('meta')
            for meta in meta_arr:
                if 'overrides' in meta:
                    merge_defs_from_symbols_as(tree_name, new_keyed, [x['sym'] for x in meta['overrides']], 'Overrides')
                if 'overriddenBy' in meta:
                    # Currently this derived relationship only includes the symbol
                    # name, as opposed to the overrides cases which is an obj with
                    # { sym, pretty }.
                    merge_defs_from_symbols_as(tree_name, new_keyed, meta['overriddenBy'], 'Overridden By')
                if 'supers' in meta:
                    merge_defs_from_symbols_as(tree_name, new_keyed, [x['sym'] for x in meta['supers']], 'Superclasses')
                if 'subclasses' in meta:
                    # This is also a derived relationship with only the symbol.
                    merge_defs_from_symbols_as(tree_name, new_keyed, meta['subclasses'], 'Subclasses')
        else:
            del new_keyed['meta']

    return new_keyed

def escape_regex(searchString):
    # a version of re.escape that doesn't escape every non-ASCII character,
    # and therefore doesn't mangle utf-8 encoded characters.
    # https://bugzilla.mozilla.org/show_bug.cgi?id=1446220
    return re.sub(r"[(){}\[\].*?|^$\\+-]", r"\\\g<0>", searchString)

def parse_search(searchString):
    pieces = searchString.split(' ')
    result = {}
    for i in range(len(pieces)):
        if pieces[i].startswith('path:'):
            result['pathre'] = parse_path_filter(pieces[i][len('path:'):])
        elif pieces[i].startswith('pathre:'):
            result['pathre'] = pieces[i][len('pathre:'):]
        elif pieces[i].startswith('context:'):
            # Require the context to be an integer <= 10.
            try:
                # This may throw.
                context_lines = int(pieces[i][len('context:'):])
                context_lines = max(0, context_lines)
                context_lines = min(10, context_lines)
                result['context_lines'] = context_lines
            except:
                pass
        elif pieces[i].startswith('symbol:'):
            result['symbol'] = ' '.join(pieces[i:])[len('symbol:'):].strip().replace('.', '#')
        elif pieces[i].startswith('re:'):
            result['re'] = (' '.join(pieces[i:]))[len('re:'):]
            break
        elif pieces[i].startswith('text:'):
            result['re'] = escape_regex((' '.join(pieces[i:]))[len('text:'):])
            break
        elif pieces[i].startswith('id:'):
            result['id'] = pieces[i][len('id:'):]
        else:
            result['default'] = escape_regex(' '.join(pieces[i:]))
            break

    return result

def is_trivial_search(parsed):
    if 'symbol' in parsed:
        return False

    for k in parsed:
        if k == 'context_lines':
            continue
        if len(parsed[k]) >= 3:
            return False

    return True

class SearchResults(object):
    def __init__(self):
        self.results = []
        self.qualified_results = []

        self.pathre = None
        self.compiled = {}

    def set_path_filter(self, path):
        if not path or path == '.*':
            self.pathre = None
            return

        try:
            self.pathre = re.compile(path, re.IGNORECASE)
        except re.error:
            # In case the pattern is not a valid RE, treat it as literal string.
            self.pathre = re.compile(re.escape(path), re.IGNORECASE)

    def add_results(self, results):
        self.results.append(results)

    def add_qualified_results(self, qual, results, modifier):
        self.qualified_results.append((qual, results, modifier))

    max_count = 1000
    max_work = 750
    path_precedences = ['normal', 'thirdparty', 'test', 'generated']
    key_precedences = ["Files", "IDL", "Definitions", "Overrides",
        "Overridden By", "Superclasses", "Subclasses", "Assignments", "Uses",
        "Declarations", "Textual Occurrences"]

    def categorize_path(self, path):
        '''
        Given a path, decide whether it's "normal"/"test"/"generated".  These
        are the 3 top-level groups by which results are categorized.

        These are hardcoded heuristics that probably could be better defined
        in the `config.json` metadata, with a means for trees like gecko to be
        able to leverage in-tree build meta-information like moz.build and the
        various mochitest.ini files, etc.
        '''
        def is_test(p):
            # Except /unit/ and /androidTest/, all other paths contain the substring 'test', so we can exit early
            # in case it is not present.
            if '/unit/' in p or '/androidTest/' in p:
                return True
            if 'test' not in p:
                return False
            return ('/test/' in p or '/tests/' in p or '/mochitest/' in p or 'testing/' in p or
                    '/jsapi-tests/' in p or '/reftests/' in p or '/reftest/' in p or
                    '/crashtests/' in p or '/crashtest/' in p or
                    '/googletest/' in p or '/gtest/' in p or '/gtests/' in p or
                    '/imptests/' in p)

        if '__GENERATED__' in path:
            return 'generated'
        elif path.startswith('third_party/'):
            return "thirdparty"
        elif is_test(path):
            return 'test'
        else:
            return 'normal'

    def compile_result(self, kind, qual, pathr, line_modifier):
        '''
        Given path-binned results of a specific analysis `kind` for a
        pretty symbol (`qual`), categorize the path into generated/test/normal
        and nest the results under a [pathkind, qkind, path] nested key
        hierarchy where the values are an array of crossref.rs `SearchResult`
        json results plus the line_modifier fixup hack.

        Path filtering requested via `set_path_filter` is performed at this
        stage.

        line_modifier is a (closed-over) fixup function that was passed in to
        add_qualified_results that's provided the given `line`.  It's only ever
        used by identifier_search in order to fixup "bounds" to compensate for
        prefix searches.
        '''
        if qual:
            qkind = '%s (%s)' % (kind, qual)
        else:
            qkind = kind

        path = pathr['path']
        lines = pathr['lines']

        pathkind = self.categorize_path(path)

        if self.pathre and not self.pathre.search(path):
            return

        # compiled is a map {pathkind: {qkind: {path: [(lines, line_modifier)]}}}
        kind_results = self.compiled.setdefault(pathkind, collections.OrderedDict()).setdefault(qkind, {})
        path_results = kind_results.setdefault(path, ([], line_modifier))
        path_results[0].extend(lines)

    def sort_compiled(self):
        '''
        Traverse the `compiled` state in `path_precedences` order, and then
        its "qkind" children in their inherent order (which is derived from
        the use of `key_precedences` by `get()`), transforming and propagating
        the results, applying a `max_count` result limit.

        Additional transformations that are performed:
        - result de-duplication is performed so that a given (path, line) tuple
          can only be emitted once.  Because of the intentional order of
          `key_precedences` this means that semantic matches should preclude
          their results from being duplicated in the more naive text search
          results.
        - line_modifier's bounds fixups as mentioned in `compile_result` are
          applied which helps the bolding logic in the display logic on the
          (web) client.
        '''
        count = 0

        line_hash = {}

        result = collections.OrderedDict()
        for pathkind in self.path_precedences:
            for qkind in self.compiled.get(pathkind, []):
                paths = list(self.compiled[pathkind][qkind].keys())
                paths.sort()
                for path in paths:
                    # see `compile_result` docs for line_modifier above.
                    (lines, line_modifier) = self.compiled[pathkind][qkind][path]
                    lines.sort(key=lambda l: l['lno'])
                    lines_out = []
                    for line in lines:
                        lno = line['lno']
                        key = (path, lno)
                        if key in line_hash:
                            continue
                        line_hash[key] = True
                        if line_modifier:
                            line_modifier(line)
                        lines_out.append(line)
                        count += 1
                        if count == self.max_count:
                            break

                    if lines_out or qkind == 'Files':
                        l = result.setdefault(pathkind, collections.OrderedDict()).setdefault(qkind, [])
                        l.append({'path': path, 'lines': lines_out})
                    if count == self.max_count:
                        break
                if count == self.max_count:
                    break
            if count == self.max_count:
                break

        return result

    def get(self, work_limit):
        '''
        Work-limiting/result-bounding logic to process the returned results,
        capping them based on some heuristics.  Limiting is performed for each
        "key" type (AKA analysis kind), with the harder result limit occurring
        in `sort_compiled` where a hard result limit `max_count` is enforced.

        See `compile_result` and `sort_compiled` for more info.
        '''
        # compile_result will categorize each path that it sees.
        # It will build a list of paths indexed by pathkind, qkind.
        # Later I'll iterate over this, remove dupes, sort, and keep the top ones.

        self.qualified_results.sort(key=lambda x: x[0])
        for kind in self.key_precedences:
            work = 0
            for (qual, results, line_modifier) in self.qualified_results:
                if work > self.max_work and work_limit:
                    log('WORK LIMIT HIT')
                    break
                for pathr in results.get(kind, []):
                    self.compile_result(kind, qual, pathr, line_modifier)
                    work += 1

            for results in self.results:
                for pathr in results.get(kind, []):
                    self.compile_result(kind, None, pathr, None)
                    work += 1

        r = self.sort_compiled()
        return r

def search_files(tree_name, path):
    pathFile = os.path.join(index_path(tree_name), 'repo-files')
    objdirFile = os.path.join(index_path(tree_name), 'objdir-files')
    try:
        # We set the locale to make grep much faster.
        results = subprocess.check_output(['grep', '-Eih', path, pathFile, objdirFile], env={'LC_CTYPE': 'C'}, universal_newlines=True)
    except subprocess.CalledProcessError:
        return []
    results = results.strip().split('\n')
    results = [ {'path': f, 'lines': []} for f in results ]
    return results[:1000]

def demangle(sym):
    try:
        return subprocess.check_output(['c++filt', '--no-params', sym], universal_newlines=True).strip()
    except subprocess.CalledProcessError:
        return sym

def identifier_search(search, tree_name, needle, complete, fold_case):
    needle = re.sub(r'\\(.)', r'\1', needle)

    pieces = re.split(r'\.|::', needle)
    # If the last segment of the search needle is too short, return no results
    # because we're worried that would return too many results.
    if not complete and len(pieces[-1]) < 3:
        return {}

    # Fixup closure for use by add_qualified_results to reduce the range of the
    # match's bounds to the prefix that was included in the search needle from
    # the full bounds of the search result.  (So if the search was "foo::bar"
    # and we matched "foo::bartab" and "foo::barhat", the idea I guess is that
    # only the "bar" portion would be highlighted assuming the bounds
    # previously were referencing "bartab" and "barhat".)
    def line_modifier(line):
        if 'bounds' in line:
            (start, end) = line['bounds']
            end = start + len(pieces[-1])
            line['bounds'] = [start, end]

    ids = identifiers.lookup(tree_name, needle, complete, fold_case)
    for (i, (qualified, sym)) in enumerate(ids):
        if i > 500:
            break

        q = demangle(sym)
        if q == sym:
            q = qualified

        results = expand_keys(tree_name, crossrefs.lookup_merging(tree_name, sym))
        search.add_qualified_results(q, results, line_modifier)

def get_json_search_results(tree_name, query):
    try:
        search_string = query['q'][0]
    except:
        search_string = ''

    try:
        fold_case = query['case'][0] != 'true'
    except:
        fold_case = True

    try:
        regexp = query['regexp'][0] == 'true'
    except:
        regexp = False

    try:
        path_filter = query['path'][0]
    except:
        path_filter = ''

    parsed = parse_search(search_string)

    # Should we just be leaving this in parsed?
    context_lines = 0
    if 'context_lines' in parsed:
        context_lines = parsed['context_lines']

    if path_filter:
        parsed['pathre'] = parse_path_filter(path_filter)

    if regexp:
        if 'default' in parsed:
            del parsed['default']
        if 're' in parsed:
            del parsed['re']
        parsed['re'] = search_string

    if 'default' in parsed and len(parsed['default']) == 0:
        del parsed['default']

    if is_trivial_search(parsed):
        results = {}
        return json.dumps(results)

    title = search_string
    if not title:
        title = 'Files ' + path_filter

    search = SearchResults()

    work_limit = False
    hit_timeout = False

    if 'symbol' in parsed:
        search.set_path_filter(parsed.get('pathre'))
        symbols = parsed['symbol']
        title = 'Symbol ' + symbols
        search.add_results(expand_keys(tree_name, crossrefs.lookup_merging(tree_name, symbols)))
    elif 're' in parsed:
        path = parsed.get('pathre', '.*')
        (substr_results, timed_out) = codesearch.search(parsed['re'], fold_case, path, tree_name, context_lines)
        search.add_results({'Textual Occurrences': substr_results})
        hit_timeout |= timed_out
    elif 'id' in parsed:
        search.set_path_filter(parsed.get('pathre'))
        identifier_search(search, tree_name, parsed['id'], complete=True, fold_case=fold_case)
    elif 'default' in parsed:
        work_limit = True
        path = parsed.get('pathre', '.*')
        (substr_results, timed_out) = codesearch.search(parsed['default'], fold_case, path, tree_name, context_lines)
        search.add_results({'Textual Occurrences': substr_results})
        hit_timeout |= timed_out
        if 'pathre' not in parsed:
            file_results = search_files(tree_name, parsed['default'])
            search.add_results({'Files': file_results})

            identifier_search(search, tree_name, parsed['default'], complete=False, fold_case=fold_case)
    elif 'pathre' in parsed:
        path = parsed['pathre']
        search.add_results({'Files': search_files(tree_name, path)})
    else:
        assert False
        results = {}

    results = search.get(work_limit)

    results['*title*'] = title
    results['*timedout*'] = hit_timeout
    return json.dumps(results)

def identifier_sorch(search, tree_name, needle, complete, fold_case):
    needle = re.sub(r'\\(.)', r'\1', needle)

    pieces = re.split(r'\.|::', needle)
    # If the last segment of the search needle is too short, return no results
    # because we're worried that would return too many results.
    if not complete and len(pieces[-1]) < 3:
        return

    # Fixup closure for use by add_qualified_results to reduce the range of the
    # match's bounds to the prefix that was included in the search needle from
    # the full bounds of the search result.  (So if the search was "foo::bar"
    # and we matched "foo::bartab" and "foo::barhat", the idea I guess is that
    # only the "bar" portion would be highlighted assuming the
    # previously were referencing "bartab" and "barhat".)
    def line_modifier(line):
        if 'bounds' in line:
            (start, end) = line['bounds']
            end = start + len(pieces[-1])
            line['bounds'] = [start, end]

    ids = identifiers.lookup(tree_name, needle, complete, fold_case)
    for (i, (qualified, sym)) in enumerate(ids):
        if i > 500:
            break

        q = demangle(sym)
        if q == sym:
            q = qualified

        sym_data = crossrefs.lookup_single_symbol(tree_name, sym)
        if sym_data:
            # XXX we could pass line_modifier here and have it be used; the
            # logic probably still holds.  OTOH, having the full symbol that
            # matched by prefix doesn't seem like the end of the world.
            search.add_symbol(sym, sym_data)

def get_json_sorch_results(tree_name, query):
    '''
    New RawSearchResults variant.  Initially supports 'symbol:', 'id:' and
    default queries that only perform identifier searches and filename searches
    (no fulltext).
    '''
    try:
        search_string = query['q'][0]
    except:
        search_string = ''

    try:
        fold_case = query['case'][0] != 'true'
    except:
        fold_case = True

    try:
        regexp = query['regexp'][0] == 'true'
    except:
        regexp = False

    try:
        path_filter = query['path'][0]
    except:
        path_filter = ''

    parsed = parse_search(search_string)

    if path_filter:
        parsed['pathre'] = parse_path_filter(path_filter)

    if regexp:
        if 'default' in parsed:
            del parsed['default']
        if 're' in parsed:
            del parsed['re']
        parsed['re'] = search_string

    if 'default' in parsed and len(parsed['default']) == 0:
        del parsed['default']

    if is_trivial_search(parsed):
        results = {}
        return json.dumps(results)

    title = search_string
    if not title:
        title = 'Files ' + path_filter

    search = RawSearchResults()

    work_limit = False
    hit_timeout = False

    if 'symbol' in parsed:
        search.set_path_filter(parsed.get('pathre'))
        symbols = parsed['symbol']
        title = 'Symbol ' + symbols
        for symbol in symbols.split(','):
            sym_data = crossrefs.lookup_single_symbol(tree_name, symbol)
            if sym_data:
                search.add_symbol(symbol, sym_data)
    elif 'id' in parsed:
        search.set_path_filter(parsed.get('pathre'))
        identifier_sorch(search, tree_name, parsed['id'], complete=True, fold_case=fold_case)
    elif 'default' in parsed:
        work_limit = True
        path = parsed.get('pathre', '.*')
        #(substr_results, timed_out) = codesearch.search(parsed['default'], fold_case, path, tree_name)
        #search.add_results({'Textual Occurrences': substr_results})
        #hit_timeout |= timed_out
        if 'pathre' not in parsed:
            file_results = search_files(tree_name, parsed['default'])
            search.add_paths(file_results)

            identifier_sorch(search, tree_name, parsed['default'], complete=False, fold_case=fold_case)
    else:
        assert False
        results = {}

    results = search.get()

    results['*title*'] = title
    results['*timedout*'] = hit_timeout
    return json.dumps(results)

class Handler(six.moves.SimpleHTTPServer.SimpleHTTPRequestHandler):
    def do_GET(self):
        pid = os.fork()
        if pid:
            # Parent process
            log('request(handled by %d) %s', pid, self.path)

            timedOut = [False]
            def handler(signum, frame):
                log('timeout %d, killing', pid)
                timedOut[0] = True
                os.kill(pid, signal.SIGKILL)
            signal.signal(signal.SIGALRM, handler)
            signal.alarm(120)

            t = time.time()
            while True:
                try:
                    (pid2, status) = os.waitpid(pid, 0)
                    break
                except OSError as e:
                    if e.errno != errno.EINTR: raise e

            failed = timedOut[0]
            if os.WIFEXITED(status) and os.WEXITSTATUS(status) != 0:
                log('error pid %d - %f', pid, time.time() - t)
                failed = True
            else:
                log('finish pid %d - %f', pid, time.time() - t)

            if failed:
                self.send_response(504)
                self.end_headers()
        else:
            # Child process
            try:
                self.process_request()
                os._exit(0)
            except:
                e = traceback.format_exc()
                log('exception\n%s', e)
                os._exit(1)

    def log_request(self, *args):
        pass

    def _wrap_sorch_results(self, tree_name, query):
        '''
        Commonalities around sorch results and URI wrappers like "symbol" that
        are just a specialized sorch.
        '''
        j = get_json_sorch_results(tree_name, query)
        if 'json' in self.headers.get('Accept', ''):
                self.generateJson(j)
        else:
            j = j.replace("</", "<\\/").replace("<script", "<\\script").replace("<!", "<\\!")
            template = os.path.join(index_path(tree_name), 'templates/sorch.html')
            self.generateWithTemplate({'{{BODY}}': j, '{{TITLE}}': 'Search'}, template)

    def process_request(self):
        url = six.moves.urllib.parse.urlparse(self.path)
        path_elts = url.path.split('/')

        # Strip any extra slashes.
        path_elts = [ elt for elt in path_elts if elt != '' ]

        if len(path_elts) >= 2 and path_elts[1] == 'search':
            tree_name = path_elts[0]
            query = six.moves.urllib.parse.parse_qs(url.query)
            j = get_json_search_results(tree_name, query)
            if 'json' in self.headers.get('Accept', ''):
                self.generateJson(j)
            else:
                j = j.replace("</", "<\\/").replace("<script", "<\\script").replace("<!", "<\\!")
                template = os.path.join(index_path(tree_name), 'templates/search.html')
                self.generateWithTemplate({'{{BODY}}': j, '{{TITLE}}': 'Search'}, template)
        elif len(path_elts) >= 2 and path_elts[1] == 'sorch':
            tree_name = path_elts[0]
            query = six.moves.urllib.parse.parse_qs(url.query)
            self._wrap_sorch_results(tree_name, query)
        # "symbol" is a variant on "define", but whereas "define" creates a
        # redirect, "symbol" is equivalent to source with "q=symbol:ORIGINAL_Q"
        elif len(path_elts) >= 2 and path_elts[1] == 'symbol':
            tree_name = path_elts[0]
            orig_query = six.moves.urllib.parse.parse_qs(url.query)
            symbol = orig_query['q'][0]
            new_query = { 'q': [ 'symbol:' + symbol ]}
            self._wrap_sorch_results(tree_name, new_query)
        elif len(path_elts) >= 2 and path_elts[1] == 'define':
            tree_name = path_elts[0]
            query = six.moves.urllib.parse.parse_qs(url.query)
            symbol = query['q'][0]
            results = expand_keys(tree_name, crossrefs.lookup_merging(tree_name, symbol), False)
            definition = results['Definitions'][0]
            filename = definition['path']
            lineno = definition['lines'][0]['lno']
            url = '/' + tree_name + '/source/' + filename + '#' + str(lineno)

            self.send_response(301)
            self.send_header("Location", url)
            self.end_headers()
        else:
            return six.moves.SimpleHTTPServer.SimpleHTTPRequestHandler.do_GET(self)

    def generateJson(self, data):
        databytes = data.encode('utf-8')

        self.send_response(200)
        self.send_header("Vary", "Accept")
        self.send_header("Content-type", "application/json;charset=utf-8")
        self.send_header("Content-Length", str(len(databytes)))
        self.end_headers()

        self.wfile.write(databytes)

    def generateWithTemplate(self, replacements, templateFile):
        output = open(templateFile).read()
        for (k, v) in replacements.items():
            output = output.replace(k, v)

        databytes = output.encode('utf-8')

        self.send_response(200)
        self.send_header("Vary", "Accept")
        self.send_header("Content-type", "text/html;charset=utf-8")
        self.send_header("Content-Length", str(len(databytes)))
        self.end_headers()

        self.wfile.write(databytes)

config_fname = sys.argv[1]
status_fname = sys.argv[2]

config = json.load(open(config_fname))

os.chdir(config['mozsearch_path'])

crossrefs.load(config)
codesearch.load(config)
identifiers.load(config)

# We *append* to the status file because other server components
# also write to this file when they are done starting up, and we
# don't want to clobber those messages.
with open(status_fname, "a") as status_out:
    status_out.write("router.py loaded\n")

class ForkingServer(ForkingMixIn, HTTPServer):
    pass

server_address = ('', 8000)
httpd = ForkingServer(server_address, Handler)
httpd.serve_forever()
