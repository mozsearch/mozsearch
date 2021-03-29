import re

class RawSearchResults(object):
    '''
    Alternate version of SearchResults that attempts to leave information in its
    underlying structured representation rather that performing consolidation
    for presentation purposes.

    The primary complication is that all of our data related to a symbol is
    actually file-centric and we want to shard these different pieces of a
    fragment by normal/test/generated.
    '''

    def __init__(self):
        self.paths = [];
        self.symbols = {}

        self.pathre = None

    # directly extracted from SearchResults
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
            # Except /unit/, all other paths contain the substring 'test', so we can exit early
            # in case it is not present.
            if '/unit/' in p:
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
        elif is_test(path):
            return 'test'
        else:
            return 'normal'

    # directly extracted from SearchResults
    def set_path_filter(self, path):
        if not path or path == '.*':
            self.pathre = None
            return

        try:
            self.pathre = re.compile(path, re.IGNORECASE)
        except re.error:
            # In case the pattern is not a valid RE, treat it as literal string.
            self.pathre = re.compile(re.escape(path), re.IGNORECASE)

    def add_paths(self, paths):
        self.paths.extend(paths)

    def add_symbol(self, raw_sym, data):
        '''
        Given a symbol in crossrefs representation, process it into our output
        representation.  This mainly means splitting `data` into
        normal/test/generated groups.
        '''
        if raw_sym in self.symbols:
            # XXX the symbol should only be added once, so ignore if it's
            # already in there.
            return

        sym_info = {}
        sym_info['symbol'] = raw_sym

        hits_by_pathkind = sym_info['hits'] = {}
        # the goal here is to go from the flattened tuple of [kind, path, lines]
        # to [pathkind, kind, path, lines].  We are able to reuse the
        # (path, lines) pair at the end, so it's really just the structure of
        # [pathkind, kind] that we need to create.
        for kind, path_line_pairs in data.items():
            if kind == 'meta':
                sym_info['meta'] = path_line_pairs
                continue
            if kind == 'consumes':
                sym_info['consumes'] = path_line_pairs
                continue
            
            for path_lines in path_line_pairs:
                path = path_lines['path']
                lines = path_lines['lines']

                # skip this path if it's filtered out.
                if self.pathre and not self.pathre.search(path):
                    continue

                pathkind = self.categorize_path(path)
                hits_by_kind = hits_by_pathkind.get(pathkind, None)
                if hits_by_kind is None:
                    hits_by_kind = hits_by_pathkind[pathkind] = {}

                kind_path_lines_list = hits_by_kind.get(kind, None)
                if kind_path_lines_list is None:
                    kind_path_lines_list = hits_by_kind[kind] = []
                # we can reuse the path_line pairs.
                kind_path_lines_list.append(path_lines)

        self.symbols[raw_sym] = sym_info

    def get(self):
        '''
        Produce a JSON-able final result.  Our current schema looks like:
        - files: List of String paths.
        - fulltext: Currently null because we don't return those results.
        - semantic: A dictionary of what we're calling SymbolResult things keyed
          by their raw symbol.

        Each SymbolResult is a dictionary that has the following members.  This
        is where additional info and meta-info about symbols will be found in
        the future.
        - symbol: The raw symbol.
        - pretty: The prettified version of the symbol.
        - hits: dict with keys:
          - normal/test/generated: (pathkind) dict with keys:
            - uses/defs/assignments/decls/idl/conumes: (kind): list of
              with keys:
              - path
              - lines:
                - lno
                - line
                - bounds
                - contextsym
                - context
                - peekLines
        '''

        results = {}
        results['files'] = self.paths
        results['semantic'] = self.symbols

        return results
