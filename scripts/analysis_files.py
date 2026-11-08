# The analysis records of the files whose analysis an analyzer reads (ex:
# idl-analyze.py reads its generated headers' C++ analysis): ANALYSIS_ROOT's,
# or, for the files it doesn't have, a base analysis's (ex: the tree's, for a
# try push's; see try-analyze-rest.sh), gzipped or not, as a web server's is
# (with an empty file of its own name); and both's for the files it has only
# some of the records of (an incomplete list, as build-try-delta's
# --incomplete).

import gzip
import os


class AnalysisFiles:
    def __init__(self, root, base=None, incomplete=None):
        self.root = root
        self.base = base
        self.incomplete = set()
        if incomplete:
            with open(incomplete) as f:
                self.incomplete = set(line.strip() for line in f if line.strip())

    def _read_base(self, path):
        gz = os.path.join(self.base, path + '.gz')
        if os.path.isfile(gz):
            with gzip.open(gz, 'rt') as f:
                return f.readlines()
        try:
            with open(os.path.join(self.base, path)) as f:
                return f.readlines()
        except IOError:
            return None

    def lines(self, path):
        '''The lines of the analysis of `path` (relative to the root), or None
        if there's none.'''
        own = os.path.join(self.root, path)
        has_own = os.path.isfile(own) and os.path.getsize(own) > 0
        lines = None
        if has_own or not self.base:
            try:
                with open(own) as f:
                    lines = f.readlines()
            except IOError:
                pass
        if self.base and (not has_own or path in self.incomplete):
            base = self._read_base(path)
            if base is not None:
                lines = (lines or []) + base
        return lines

    def listdir(self, path):
        '''The names in the directory `path` (relative to the root), in either
        analysis.'''
        names = set()
        for root in [self.root, self.base]:
            if root and os.path.isdir(os.path.join(root, path)):
                names.update(os.listdir(os.path.join(root, path)))
        return sorted(names)
