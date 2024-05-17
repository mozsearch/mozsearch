# Index Directory Contents

If you look inside an index directory (ex: `~/index/mozilla-central` on an AWS
server), these are the files and sub-trees you may find, and who put them there.
Simpler configurations with fewer platforms will be simpler.

Note that many of the files referenced here are from the
https://github.com/mozsearch/mozsearch-mozilla configuration repository, most
specifically referencing the "mozilla-central" tree in its config1.json.

Directories:
- `analysis`: Directory hierarchy that directly corresponds to the paths exposed
  by the searchfox UI as a single unified namespace where objdir files are
  folded into `__GENERATED__`.  Each file has the name of its corresponding
  source/generated file and contains JSON analysis data.  It is populated by
  the indexing process.  For mozilla-central and similar builds, some of the
  indexing (ex: C++) occurs on the taskcluster build machine and is inherently
  per-platform, with `process-gecko-analysis.sh` using the per-platform
  `process-tc-artifacts.sh` to process the per-platform data and then
  `collapse-generated-files.sh` and `merge-analyses.rs` to merge the
  per-platform data into merged analysis files.
- `description`: Searchfox Directory hierarchy with per-file text files that
  contain extracted summaries from files extracted by `describe.rs` using
  heuristics that usually involve extracting the contents of comments found
  in the file.  These summaries are produced by `output-file.rs` as a
  byproduct of writing the HTML for the files to disk during the
  `output.sh` stage of `mkindex.sh`.
- `dir`: HTML files for the directory listings for each file.  It forms a
  parallel hierarchy to the `file` directory.  This is necessary because the
  directory HTML files are placed at `index.html` inside each directory which
  would collide if there is a source file with that name.  The nginx config
  uses a lookup sequence to make this work.  Produced by `output-dir.js`.
- `file`: HTML files for each source/generated file.  Produced by
  `output-file.rs` from the source/generated file itself, the corresponding
  analysis file found under `analysis/`, the `jumps` all-files aggregate file,
  the `derived-per-file-info.json` all-files aggregate file, the
  corresponding per-file aggregate file found under `per-file-info/`.
- `gecko-blame`: git repository containing pre-computed per-file blame/annotate
  info built by `build-blame.rs`.  The directory name is specific per repository
  configuration.
- `gecko-dev`: Source git repository (using git-cinnabar).  The directory name
  is specific per repository configuration.  The indexed revision will be the
  currently checked out working directory.
- `objdir`: The conceptual source directory for things found under
  `__GENERATED__` in Searchfox's unified namespace.  When `output-file.rs` is
  generating an HTML file for `__GENERATED__/foo`, `foo` will be found under
  this directory.
- `objdir-*`: Leftover per-platform files from `process-gecko-analysis.sh`,
  probably just rust "save-analysis" files.  Most files are destructively
  consumed or moved during the process.  These leftovers are retained for
  debugging purposes.
- `detailed-per-file-info`: Directory hierarchy like `analysis` for storing
  detailed per-file information in a JSON file that's too large to put in the
  single aggregate `concise-per-file-info.json` file or not useful for summary
  purposes.
- `templates`: Holds the result of `output-template.js` which builds
  `search.html` using the hardcoded HTML generation logic in `output-lib.js` and
  `output.js` to build the searchfox UI scaffolding and save it so that
  `router.py` can inline the JSON search results from a query.

Files:
- `all-dirs`: `repo-dirs` and `objdir-dirs` concatenated together by
  `find-objdir-files.sh` after deriving `objdir-dirs`.  Exists for the benefit
  of crossref for now.
- `all-files`: `repo-files` and `objdir-files` concatenated together by
  `find-objdir-files.sh` and shuffled after deriving `objdir-files`.  This
  exists so that `output.sh` can use `--pipe-part` which needs a real file on
  disk rather than a pipe from dynamically `cat`-ing the source files.  Also now
  the crossref script consumes this file.
- `analysis-dirs-*.list`: `find -type d` for each per-platform analysis
  directory.  Produced by the per-platform `process-tc-artifacts.sh` script and
  concatenated into the unified list by `process-gecko-analysis.sh`.  The
  paths are all relative to the per-platform directory and so don't actually
  include the platform-specific path.
- `analysis-dirs.list`: A concatenated list of the above per-platform lists
  created by `process-gecko-analysis.sh` and unique-ified, used to ensure the
  appropriate directories are created in the `analysis/` tree for the script's
  invocation of `merge-analyses` with stdout redirection (which can't mkdir -p
  itself).
- `analysis-files-*.list`: `find -type f` for each per-platform analysis
  directory.  Produced by the per-platform `process-tc-artifacts.sh` script and
  concatenated into the unified list by `process-gecko-analysis.sh`.  The
  paths are all relative to the per-platform directory and so don't actually
  include the platform-specific path.
- `analysis-files.list`: A concatenated list of the above per-platform lists
  created by `process-gecko-analysis.sh` and unique-ified and then passed to a
  `parallel` invocation of `merge-analyses` with stdout redirection.  Note that
  generated files are handled separately and use `generated-files.list`.
- `android-armv7.*`: A bunch of per-platform files downloaded by
  `fetch-tc-artifacts.sh` that we retain for debugging
  `process-gecko-analysis.sh`.
- `bugzilla-components.json`: Downloaded by `fetch-tc-artifacts.sh` and
  integrated into per-file information by `derive-per-file-info.rs` when invoked
  by `crossref.sh`.
- `crossref`: The big database produced by `crossref.rs` that has all the
  per-symbol information that gets returned by (symbol) search results by
  `router.py` after first mapping from pretty human names to machine symbol
  names using `identifiers`.  See [crossref.md](crossref.md) for more info.
- `concise-per-file-info.json`: Produced by `derive-per-file-info.rs` when
  invoked by `crossref.sh`.
- `downloads.lst`: List of curl download commands accumulated by
  `fetch-tc-artifacts.sh` so that it can run them in parallel.
- `generated-files-*.list`: `find -type f` for each per-platform generated-files
  directory.  Produced by the per-platform `process-tc-artifacts.sh` script and
  concatenated into the unified list by `process-gecko-analysis.sh`.  The
  paths are all relative to the per-platform directory and so don't actually
  include the platform-specific path.
- `generated-files.list`:  A concatenated list of the above per-platform lists
  created by `process-gecko-analysis.sh` and unique-ified and then passed to a
  `parallel` invocation of `collapse-generated-files.sh` which takes on
  responsibility for running `mkdir -p` directly and so doesn't need a `-dirs`
  variant of this list.
- `help.html`: The file you see at the root of the searchfox UI that is
  basically the HTML contents of the config repo's `help.html` with all the
  searchfox UI scaffolding wrapped around it by `output-help.js` which uses
  `output-lib.js` and `output.js` just like is done for `templates/search.html`.
- `identifiers`: A text file mapping pretty human-readable symbol names to
  machine-readable (AKA mangled C++) symbol names.  Generated by `crossref.rs`
  and part of `router.py`'s search logic.  See [crossref.md](crossref.md) for
  more info.
- `idl-files`: A list of all the '.idl' files in the tree produced by
  `find-repo-files.py` found and that the per-config `repo_files.py` didn't
  veto.  Used by `idl-analyze.sh` to know what files to process when invoked by
  `mkindex.sh`.
- `ipdl-files`: A list of all the '.ipdl' files in the tree produced by
  `find-repo-files.py` found and that the per-config `repo_files.py` didn't
  veto.  Used by `ipdl-analyze.sh` to know what files to process when invoked by
  `mkindex.sh`.
- `ipdl-includes`: A list of all the '.ipdlh' files in the tree produced by
  `find-repo-files.py` found and that the per-config `repo_files.py` didn't
  veto.  Used by `idl-analyze.sh` to know what files to process when invoked by
  `mkindex.sh`.
- `js-files`: A list of all the '.js' files in the tree produced by
  `find-repo-files.py` found and that the per-config `repo_files.py` didn't
  veto.  Used by `js-analyze.sh` to know what files to process when invoked by
  `mkindex.sh`.
- `jumps`: Lookup table that maps from machine symbol names to their canonical
  definition point.  Produced by `crossref.rs` and consumed by `output-file.rs`
  so that the context menus can in the HTML files can generate definition links
  without having to involve any server queries.  See [crossref.md](crossref.md)
  for more info.
- `linux64.*`: A bunch of per-platform files downloaded by
  `fetch-tc-artifacts.sh` that we retain for debugging
  `process-gecko-analysis.sh`.
- `livegrep.idx`: This is the output file generated by the `codesearch`
   invocation in `build-codesearch.py` and contains the full-text index that the
    `codesearch` tool uses to do full-text search. It gets loaded by the
    `codesearch` invocation on the web-server instance, in
    `router/codesearch.py`.
- `macosx64.*`: A bunch of per-platform files downloaded by
  `fetch-tc-artifacts.sh` that we retain for debugging
  `process-gecko-analysis.sh`.
- `macosx64-aarch64.*`: A bunch of per-platform files downloaded by
  `fetch-tc-artifacts.sh` that we retain for debugging
  `process-gecko-analysis.sh`.
- `objdir-dirs`: A list of the directories found under `objdir/` for scripting
  and indexing purposes using in a bunch of places.  This is necessary because
  source files are exposed via the UI at `/PATH` and come from `gecko-dev/PATH`
  and generated files are exposed at `/__GENERATED__/PATH` and come from
  `objdir/PATH`.  Produced by `find-objdir-files.sh` which is invoked by
  `mkindex.sh` early in the indexing process.
- `objdir-files`: File variant of `objdir-dirs`, see above for more info.
- `repo-dirs`: A list of the directories that correspond to source files tracked
  by revision control produced by `find-repo-files.py` which actually runs
  `git ls-files` so if you don't check your files into git they won't show up.
  As with `objdir-dirs`, this needs to exist because of the split between source
  files and generated files.
- `repo-files`: File variant of `repo-dirs`, see above for more info.
- `target.json`: Downloaded by `resolve-gecko-revs.sh` as part of the process
  of identifying the most recent successful searchfox indexing jobs run on
  taskcluster for the given channel/tree.  Taskclusters' routes mechanism means
  that the most recent job will be exposed via both its specific revision and
  "latest", so if we fetch the "latest" version, we'll get a real revision in
  this JSON file and can then use it to make sure all other fetched results come
  from the exact same revision.
- `test-info-all-tests.json`: Downloaded by `fetch-tc-artifacts.sh` and
  integrated into per-file information by `derive-per-file-info.rs` when invoked
  by `crossref.sh`.
- `win64.*`: A bunch of per-platform files downloaded by
  `fetch-tc-artifacts.sh` that we retain for debugging
  `process-gecko-analysis.sh`.
- `wpt-metadata-summary.json`: Downloaded by `fetch-tc-artifacts.sh` and
  integrated into per-file information by `derive-per-file-info.rs` when invoked
  by `crossref.sh`.
