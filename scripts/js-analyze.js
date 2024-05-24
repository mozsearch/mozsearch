let nextSymId = 0;
let localFile, sourcePath, fileIndex, mozSearchRoot;

// The parsing mode we're currently using.
let gParsedAs = "script";
// Filename for logError to use heuristics to downgrade errors/warnings.
let gFilename = "";
// Was there an `#include` present which should downgrade errors/warnings?
let gIncludeUsed = false;
// Was the first character of the file `{`?
let gCouldBeJson = false;
// Attribute name for logError to use heuristics to downgrade errors.
let gAttrName = "";

const ERROR_INTERVENTIONS = [
  {
    includes: "expected expression, got '<': ",
    severity: "INFO",
    prepend: "React detected: ",
  },
  {
    includes: "import assertions are not currently supported: ",
    severity: "INFO",
    prepend: "Not yet supported: "
  },
  {
    includes: "illegal character",
    severity: "INFO",
    prepend: "Illegal characters are probably intentional: "
  },
  {
    includes: "invalid escape sequence:",
    severity: "INFO",
    prepend: "Invalid escapes are probably intentional: "
  },
  {
    includes: "missing ; after for-loop condition: ",
    severity: "INFO",
    prepend: "Wacky test idiom?: "
  },
  {
    includes: "expected expression, got '%'",
    severity: "INFO",
    prepend: "Probable WPT interpolation mechanism: "
  },
  // This happened on `import("./basic.css", { assert: { type: "css" } })` in
  // a WPT only in esr91 where it seems like dynamic import is hard-coded to
  // not know about the second optional args right now.  However, we do seem to
  // be implementing the assertions, so this could go away.
  //
  // That said, this type of problem in the JS code is not something searchfox
  // can do anything about, especially if our parser is mad, so this is
  // reasonable to downgrade in general.
  {
    includes: "missing ) after argument list",
    severity: "INFO",
    prepend: "(unsupported) import assertions can parse this way: "
  },
  // Another new variation on import assertions for CSS module assertions
  //
  // web-platform/tests/html/semantics/scripting-1/the-script-element/css-module-assertions/resources/integrity-matches.js:1
  // because SyntaxError: unexpected token: 'assert'
  {
    includes: "unexpected token: 'assert'",
    severity: "INFO",
    prepend: "(unsupported) import assertions can also parse this way: "
  },
  // This warning started appearing after https://bugzilla.mozilla.org/show_bug.cgi?id=1845085
  // which imported some new web-platform tests with a new import syntax that looks
  // like `import "./hello.js#3" with { type: "js" };`. It appears that this syntax is not
  // yet supported by our parser but may be supported in the future.
  {
    includes: "unexpected token: keyword 'with'",
    severity: "INFO",
    prepend: "(unsupported) import attributes can parse this way: "
  },
  {
    includes: "redeclaration of import",
    severity: "INFO",
    prepend: "Known buggy code pattern is not a problem: "
  },
  // Bug 1858251 landed https://github.com/web-platform-tests/wpt/pull/42467
  // which tests syntax changes from https://github.com/tc39/proposal-source-phase-imports
  // which SpiderMonkey doesn't understand yet.
  {
    includes: "after import clause",
    severity: "INFO",
    prepend: "(unsupported) source phase imports can parse this way: "
  }
];

// Note that once we can process .eslintignore most of these can go away because
// the heroic work of people like :Standard8 making eslint work means that we
// don't need hacky heuristics like this.
//
// Note: ALL INCLUDES MUST BE LOWERCASED because that's what we match against.
const FILENAME_INTERVENTIONS = [
  {
    // "dromaeo" is from:
    // https://searchfox.org/mozilla-central/source/testing/talos/talos/tests/dromaeo/test-tail.js
    //
    // dom/media/test/test_imagecapture.html had syntax error which is fixed in trunk.
    //
    // "JSTests" is wubkat.
    includes_list: ["error", "fixture", "bad", "syntax", "invalid", "dromaeo", "/jstests/",
                    "test_bug531176.html", "dom/media/test/test_imagecapture.html"],
    severity: "INFO",
    // JS engines love to have test cases that intentionally have syntax errors
    // in them.  To this end, we downgrade any such file to an info.  This
    // undoubtedly will catch some false positives but the warning mechanism
    // is about systemic issues in analysis, so we'd expect to have reports from
    // files that don't get caught by this too in that case.
    prepend: "It may intentionally be illegal JS: ",
  },
  {
    includes_list: ["parser/htmlparser/tests"],
    severity: "INFO",
    prepend: "It may be testing script tag handling: ",
  },
  {
    // Session Store has some JSON files with .js extensions.  .eslintignore
    // does already know about them, but until my work on file ingestion lands
    // we lack an easy way to filter the JS ingestion set.
    //
    // https://bugzilla.mozilla.org/show_bug.cgi?id=1792369 was filed to track
    // fixing that and we can remove this once it's fixed.
    //
    // Argh, there's actually the following too:
    // https://searchfox.org/mozilla-central/source/testing/talos/talos/startup_test/sessionrestore/profile-manywindows/sessionstore.js
    //
    // Okay, also adding "json" for
    // `dom/tests/mochitest/ajax/jquery/test/data/json_obj.js` which reports a
    // `SyntaxError: unexpected token: ':'` which could jointly be considered
    // but there may be other variations.
    //
    // Note: I've now added the gCouldBeJson mechanism which could perhaps moot
    // the need for this intervention, but I'm out of time for the day and don't
    // want to experiment with this at the risk of re-introducing warnings.
    includes_list: ["sessionstore", "json"], // be resistant to directory hierarchy changes
    severity: "INFO",
    prepend: "Could be a JSON file based on the name: ",
  },
  // mozbuild has some weird JS looking files that are not JS:
  // https://searchfox.org/mozilla-central/search?q=python%2Fmozbuild%2Fmozbuild%2Ftest%2Fbackend%2Fdata%2Fbuild%2Fbar.js&path=
  {
    includes_list: ["mozbuild"],
    severity: "INFO",
    prepend: "mozbuild has weird files: ",
  },
  {
    // "devtools/client/shared/vendor/jszip.js" is a UMD that makes things
    // angry and it's believable we could have a bunch of these.
    //
    // also: third_party/libwebrtc/tools/grit/grit/testdata/test_js.js
    includes_list: ["vendor", "third_party"],
    severity: "INFO",
    prepend: "Vendored files can be weird: ",
  },
  {
    // pref files can be weird / annoying, ex:
    // https://searchfox.org/mozilla-central/source/testing/condprofile/condprof/tests/profile/user.js
    // there's also things like `channel-prefs.js`, so I've decided not to
    // include a slash before either of these.
    //
    // Also explicitly excluding the libpref tree for
    // `modules/libpref/test/unit/data/testParser.js`.
    //
    // Also mozprofile has `prefs_with_comments.js` and could gain others.
    //
    // https://searchfox.org/l10n/source/tn/mail/all-l10n.js is an l10n example
    // of a pref file; there are others like `firefox-l10n.js`.
    includes_list: ["user.js", "prefs.js", "firefox.js", "/libpref/", "/mozprofile/", "-l10n.js"],
    severity: "INFO",
    prepend: "Prefs files can be weird: ",
  },
  {
    // `js/src/devtools/rootAnalysis/build.js` is a new thing but it was also
    // a case where a mozconfig had a .js syntax.
    includes_list: ["/rootanalysis/"], // lowercased to match the subject
    severity: "INFO",
    prepend: "rootAnalysis does some weird custom stuff: "
  },
  {
    // There are ton of things that are clearly templating under
    // toolkit/components/uniffi-bindgen-gecko-js/src/templates
    includes_list: ["template"],
    severity: "INFO",
    prepend: "May be templated JS: "
  },
  {
    // there are a bunch of things that make us sad under tools/lint/test/files/
    includes_list: ["lint"],
    severity: "INFO",
    prepend: "May be a linting test case: ",
  },
  {
    // testing/mochitest/MochiKit/Controls.js:578 is missing a close paren
    includes_list: ['/mochikit/'], // lowecased to match the subject
    severity: "INFO",
    prepend: "Legacy weird MochiKit stuff: ",
  },
  // testing/web-platform/meta/screen-wake-lock/wakelock-insecure-context.any.js
  // is an example of a file where a typo left off the ".ini" suffix.  There's
  // never going to be any actual JS under the meta dir.
  {
    includes_list: ["testing/web-platform/meta/"],
    severity: "INFO",
    prepend: "Someone forgot to add an .ini suffix: ",
  },
  {
    // .sub.js is an explicit WPT (idiom?) thing but there are permutations so
    // we need to just detect on `.sub.`, like `.sub.window.js`, and
    // `.sub.h2.any.js`.

    // `testing/web-platform/tests/cors/support.js` is also using the
    // replacement mechanism but doesn't follow the idiom.
    includes_list: [".sub.", "/support.js"],
    severity: "INFO",
    prepend: "Substitution JS files are usually not legal JS on their own: ",
  },
  // testing/web-platform/tests/html/semantics/scripting-1/the-script-element/import-assertions/dynamic-import-with-assertion-argument.any.js
  // is an example where we get a `missing ) after argument list` instead of the
  // explicit lack of support error.
  // There are also some other cases under "/json-module/" where "json" seems to
  // save us.
  {
    includes_list: ["/import-assertions/"],
    severity: "INFO",
    prepend: "Import assertions not yet supported and may parse weird: ",
  },
  {
    // There's a bunch of syntax errors in suite code; this should ideally be
    // handled via a repo settings.  I had made this specific to comm-central
    // at first but we have ESR versions we index too, so this is now more
    // general.
    includes_list: ["/suite/"],
    severity: "INFO",
    prepend: "Unmaintained code: "
  },
  {
    // https://searchfox.org/mozilla-vpn-client/source/glean/org/mozilla/Glean/glean.js
    // is apparently a QML js file that uses a weird ".import" and ".pragma"
    // syntax that's not legit JS, obviously.
    includes_list: ["glean.js"],
    severity: "INFO",
    prepend: "May be weird QML file: "
  },
  {
    includes_list: ["/puppeteer/"],
    severity: "INFO",
    prepend: "Puppeteer has weird JS in old m-c trees: "
  },
  {
    // dom/base/crashtests/1822717-module.js is an example.
    includes_list: ["/crashtests/"],
    severity: "INFO",
    prepend: "Crashtest may intentionally contain syntax errors: "
  },
  {
    // mozilla-central proper provides the coverage we need, whereas we have an
    // ever-growing list of ESR JS code that never gets updated.  These are
    // being added for config3 which is home to our oldest ESR code.
    includes_list: ["/mozilla-esr", "/comm-esr"],
    severity: "INFO",
    prepend: "ESR failsafe: "
  },
  {
    // We still have a lot of wubkat warnings.  The is a bulk silencing, but
    // patches would be accepted to eliminate this in conjunction with the
    // addition of more specific interventions.
    //
    // I'm not adding more interventions for this right now due to:
    // - time limitations
    // - the potential for .eslintignore hook-up to perhaps moot all of these
    //   interventions
    // - our plan to replace this file with scip-typescript; our reason for
    //   logging any warnings here is to make sure we don't have coverage gaps,
    //   but when we move to scip-typescript, the quality assurance comes from
    //   scip-typescript itself, not us.
    includes_list: ["/wubkat/"],
    severity: "INFO",
    prepend: "Wubkat failsafe: "
  }
];

function logError(msg)
{
  // We log "errors" as warnings so the searchfox warning script will report it.
  let severity = "WARN";

  // But we also have some heuristics defined above that let us downgrade
  // expected problems to INFO.  Ideally these would be logged as diagnostic
  // records as proposed at https://bugzilla.mozilla.org/show_bug.cgi?id=1789515
  // but our expected migration to scip-typescript means it's probably not worth
  // it at this time, or at least not until we have the rest of the
  // diagnostic analysis record mechanism implemented.
  for (const intervention of ERROR_INTERVENTIONS) {
    if (msg.includes(intervention.includes)) {
      severity = intervention.severity;
      msg = "Downgrading warning to info because: " + intervention.prepend + msg;
      break;
    }
  }

  outer: for (const intervention of FILENAME_INTERVENTIONS) {
    let file_lower = gFilename?.toLowerCase();
    for (const include_entry of intervention.includes_list) {
      if (file_lower?.includes(include_entry)) {
        severity = intervention.severity;
        msg = "Downgrading warning to info because: " + intervention.prepend + msg;
        break outer;
      }
    }
  }

  if (gAttrName && (gFilename.includes("/test/") || gFilename.includes("/tests/"))) {
    severity = "INFO";
    msg = "Downgrading warning to info because attributes can sometimes contain syntax error in tests";
  }

  // https://searchfox.org/mozilla-central/source/browser/components/enterprisepolicies/schemas/schema.jsm
  // is an example of a file that does `const schema =` and then the next line
  // is an include and since we don't actually include things, things can break.
  // An enhancement would be accepted to try and do better, but this can't be a
  // supported feature at this time without a maintainer for it.
  if (severity === "WARN" && gIncludeUsed) {
    severity = "INFO";
    msg = `Downgrading warning to info because #include was used: ${msg}`;
  }
  if (severity === "WARN" && gCouldBeJson && msg.includes("SyntaxError: unexpected token")) {
    severity = "INFO";
    msg = `Downgrading warning to info because file could be JSON because it starts with '{': ${msg}`;
  }

  // This means we may end up needing to add a bunch of tree-specific
  // exclusions, which is probably fine.
  printErr(`${severity} when parsing as '${gParsedAs}': ${msg}\n`);
}

function SymbolTable()
{
  this.table = new Map();
}

SymbolTable.prototype = {
  put(name, symbol) {
    this.table.set(name, symbol);
  },

  get(name) {
    return this.table.get(name);
  },
};

SymbolTable.Symbol = function(name, loc)
{
  this.name = name;
  this.loc = loc;
  this.id = fileIndex + "-" + nextSymId++;
  this.uses = [];
  this.skip = false;
}

SymbolTable.Symbol.prototype = {
  use(loc) {
    this.uses.push(loc);
  },
};

function isSameLocation(loc1, loc2) {
  return loc1.start.line == loc2.start.line &&
    loc1.start.column == loc2.start.column &&
    loc1.end.line == loc2.end.line &&
    loc1.end.column == loc2.end.column;
}

function posBefore(pos1, pos2) {
  return pos1.line < pos2.line ||
         (pos1.line == pos2.line && pos1.column < pos2.column);
}

function locBefore(loc1, loc2) {
  return posBefore(loc1.start, loc2.start);
}

function locstr(loc)
{
  // mozsearch token columns are 0-based but SpiderMonkey's are now 1-based since
  // bug 1862692.
  return `${loc.start.line}:${loc.start.column - 1}`;
}

function locstr2(loc, str)
{
  // mozsearch token columns are 0-based but SpiderMonkey's are now 1-based since
  // bug 1862692.
  return `${loc.start.line}:${loc.start.column - 1}-${loc.start.column - 1 + str.length}`;
}

function locstrFull(startPos, endPos)
{
  // mozsearch token columns are 0-based but SpiderMonkey's are now 1-based since
  // bug 1862692.
  return `${startPos.line}:${startPos.column - 1}-${endPos.line}:${endPos.column - 1}`;
}

/**
 * Given an ESTree node, return true if it's potentially something that should
 * generate a nestingRange.  For our purposes, this means something that has
 * curly braces and is likely to span more than a single line of text.
 *
 * In the future this method might need to return the appropriate Location to
 * use rather than a boolean.  Right now the caller is expected to use the `loc`
 * of the provided node if we return true.
 */
function isNestingNode(node) {
  if (!node || !node.type) {
    return false
  }

  switch (node.type) {
    case "BlockStatement":
    case "FunctionExpression":
    case "ObjectExpression":
    case "ObjectPattern":
      return true;
    default:
      return false;
  }
}

function nameValid(name)
{
  if (!name) {
    return false;
  }
  for (var i = 0; i < name.length; i++) {
    var c = name.charCodeAt(i);
    switch (c) {
      case 0:  // '\0'
      case 10: // '\n'
      case 13: // '\r'
      case 32: // ' '
      case 34: // '"'
      case 92: // '\\'
        return false;
    }

    // If we have a Unicode surrogate character, make sure
    // it is a part of a valid surrogate pair, otherwise return false.

    if (c < 0xD800) {
      // Optimize common case
      continue;
    }
    if (c <= 0xDBFF && i + 1 < name.length) {
      // c is a high surrogate, check to make sure next char is a low surrogate
      var d = name.charCodeAt(i + 1);
      if (d >= 0xDC00 && d <= 0xDFFF) {
        // valid; skip over the pair and continue
        i++;
        continue;
      }
    }
    // fail on any surrogate characters that weren't part of a pair
    if (c <= 0xDFFF) {
      return false;
    }
  }
  return true;
}

function memberPropLoc(expr)
{
  // XXX this seems sketchy in terms of seeming like it thinks it is performing
  // a copy followed a mutation but that's not what is happening.  However, this
  // code is from the initial landing of searchfox so I'm not touching it right
  // now.
  let idLoc = expr.loc;
  idLoc.start.line = idLoc.end.line;
  // (we do not change the 1-base column to a 0-based column here; that will
  // happen in locstr2)
  idLoc.start.column = idLoc.end.column - expr.property.name.length;
  return idLoc;
}

function atEscape(text) {
  return text.replace(/[^A-Za-z0-9_/]/g, matched => "@" + matched.charCodeAt(0).toString(16).toUpperCase().padStart(2, "0"));
}

/**
 * Stateful singleton that assumes this script is run once per file.  General
 * structure is a imperative, recursive traversal of the
 * available-in-its-entirety JS AST.  There isn't really any streaming
 * processing and everything is kept on the stack.
 *
 * XBL is a special-case via `XBLParser`.  It is dealing with single atomic
 * chunks of JS that exist in namespace
 */
let Analyzer = {
  /**
   * The symbol table for the current scope.  When `enter` is invoked, the
   * current `symbols` table is pushed onto `symbolTableStack` and a new
   * SymbolTable is created and assigned to `symbols`.  When `exit` is invoked,
   * the current `symbols` table is discarded and replaced by popping
   * `symbolTableStack`.
   */
  symbols: new SymbolTable(),
  /**
   * Stack of `SymbolTable` instances corresponding to scopes that are reachable
   * from the current scope.  Does not include the immediate scope which is
   * found in `symbols`.
   */
  symbolTableStack: [],

  /**
   * Tracks the name of the current variable declaration so that qualified names
   * can be inferred.  When nesting occurs, the previous value is saved off on
   * the stack while call to recursive AST traversal occurs, and is restored on
   * the way out.  No attempt is currently made to infer deeply nested names,
   * just a single level, so this works as long as that assumption is okay.
   * (Note however that `contextStack` does track this nesting.)
   *
   * Specialization occurs for cases like "prototype".
   */
  nameForThis: null,
  /**
   * Tracks explicit ES "class" names.  As with `nameForThis`, nesting happens
   * on the stack so that context isn't lost, but those names are ignored for
   * symbol naming purposes.  (Note however that `contextStack` does track this
   * nesting.)
   */
  className: null,
  /**
   * Used to derive the "context" property for target records.  Whenever
   * `symbolTableStack`, `nameForThis`, or `className` are modified, the name
   * (possibly falsey) that is being used for the thing is pushed.  When
   * traversing an ObjectExpression or ObjectPattern, the key is also pushed.
   * (Object "dictionaries" like `{ a: { b: 1 } }` create a name hierarchy for
   * "a.b" but do not create lexical scopes on their own.)
   */
  contextStack: [],

  // Program lines.  Initialized by parse.  Used for getting back to program
  // source given a SourceLocation/Position.  For JS files, this should be
  // populated once.  For XUL/XBL files that invoke parse() multiple times with
  // a new, non-consecutive `line` each time, the missing lines are padded out
  // with empty strings.
  _lines: [],

  /**
   * Given a position, find the first instance of the given string starting
   * after the (exclusive, end) position.
   */
  findStrAfterPosition(str, pos) {
    // (lines are 1-based)
    let lineText = this._lines[pos.line - 1];
    if (!lineText) {
      return null;
    }
    // indexOf uses a 0-based position whereas column is 1-based but also
    // intended to be exclusive, so we subtract 1 off.
    let idx = lineText.indexOf(str, pos.column - 1);
    if (idx === -1) {
      return null;
    }
    return {
      line: pos.line,
      column: idx
    };
  },

  /**
   * If you've got some kind of outerNode like a ClassStatement where the left
   * brace comes after a node like its "id" node, use this.  The outerNode's
   * position gives the end Location and the first { found after the idNode
   * gives the start.  (Note that the end location is still chosen to be after
   * the right brace for consistency with BlockStatements.)
   */
  deriveLocationFromOuterNodeAndIdNode(outerNode, idNode) {
    let start = this.findStrAfterPosition('{', idNode.loc.end);
    if (!start) {
      return null;
    }

    return {
      start,
      end: outerNode.loc.end
    };
  },

  /**
   * Enter a new lexical scope, pushing both a new SymbolTable() to track
   * symbols defined in this scope, as well as pushing onto the contextStack
   * for "context" attribute generation purposes.
   */
  enter(name) {
    this.symbolTableStack.push(this.symbols);
    this.symbols = new SymbolTable();

    this.contextStack.push(name);
  },

  exit() {
    let old = this.symbols;
    this.symbols = this.symbolTableStack.pop();
    this.contextStack.pop();
    return old;
  },


  isToplevel() {
    return this.symbolTableStack.length == 0;
  },

  /**
   * Syntactic sugar helper to enter(name) the (potentially falsey) named
   * lexical scope, invoke the provided helper, then exit() the scope off the
   * scope/context stack.
   */
  scoped(name, f) {
    this.enter(name);
    f();
    this.exit();
  },

  get context() {
    return this.contextStack.filter(e => !!e).join(".");
  },

  dummyProgram(prog, args) {
    let stmt = prog.body[0];
    let expr = stmt.expression;

    for (let {name, skip} of args) {
      let sym = new SymbolTable.Symbol(name, null);
      sym.skip = true;
      this.symbols.put(name, sym);
    }

    if (expr.body.type == "BlockStatement") {
      this.statement(expr.body);
    } else {
      this.expression(expr.body);
    }
  },

  parse(text, filename, line, target, attrName="") {
    gAttrName = attrName;

    let ast;
    try {
      gParsedAs = target;
      try {
        ast = Reflect.parse(text, { loc: true, source: filename, line, target: gParsedAs });
      } catch (ex) {
        // If we were trying to parse something as script and it had an import,
        // attempt to re-parse it as a module.
        if ((ex.message.includes("import declarations may only appear") ||
             ex.message.includes("export declarations may only appear") ||
             // await is valid at the top-level in modules, so re-parse as a
             // module in this case too
             ex.message.includes("await is only valid in") ||
             ex.message.includes("import.meta may only appear in a module") ||
             text.includes("await import")) &&
            gParsedAs === "script") {
          gParsedAs = "module";
          ast = Reflect.parse(text, { loc: true, source: filename, line, target: gParsedAs });
        } else {
          // just re-throw because it didn't seem to be an import error.
          throw ex;
        }
      }

      let parsedLines = text.split('\n');

      if (line === 1) {
        this._lines = parsedLines;
      } else {
        // In the case of XUL/XBL, we are given random (processed) excerpts of
        // JS code with `line` representing the first line in the XML file where
        // the JS was sourced from.
        //
        // As such, we need to grow the array and insert the parsed lines so
        // that when we lookup the source JS from the AST the lines line up.
        let linesToInsert = line - this._lines.length - 1;
        while (linesToInsert-- > 0) {
          this._lines.push('');
        }
        this._lines.push(...parsedLines);
      }

    } catch (e) {
      const maybeAttr = attrName ? ` ${attrName.toLowerCase()} attribute` : '';
      logError(`Unable to parse JS file ${filename}:${line}${maybeAttr} because ${e}: ${e.fileName}:${e.lineNumber}`);
      return null;
    }
    return ast;
  },

  program(prog) {
    for (let stmt of prog.body) {
      this.statement(stmt);
    }
  },

  // maybeNesting allows passing a SourceLocation directly or a Node.  The node
  // is tested via a call to `isNestingNode` to determine whether it's an
  // appropriate type for its `loc` to be used.  This allows callers to pass
  // nodes without first checking their type.
  source(loc, name, syntax, pretty, sym, no_crossref, maybeNesting) {
    let locProp;
    if (typeof(loc) == "object" && "start" in loc) {
      locProp = locstr2(loc, name);
    } else {
      locProp = loc;
    }
    let obj = {loc: locProp, source: 1, syntax, pretty, sym};
    if (no_crossref) {
      obj.no_crossref = 1;
    }
    if (maybeNesting) {
      let nestLoc;
      if (maybeNesting.start) {
        nestLoc = maybeNesting;
      } else if (isNestingNode(maybeNesting)) {
        nestLoc = maybeNesting.loc;
      }
      if (nestLoc) {
        // substract 1 off the end column so that it points at a
        // closing brace rather than just beyond the closing brace.  This is desired for
        // the nestingRange where the goal is to reference the opening and closing
        // brace tokens directly.
        let adjustedEnd = { line: nestLoc.end.line, column: nestLoc.end.column };
        adjustedEnd.column--;
        // Handle the case where we wrap to a previous line as well, ensuring we
        // don't wrap backwards past the start position.
        while (adjustedEnd.column < 0 && posBefore(nestLoc.start, adjustedEnd)) {
          adjustedEnd.line--;
          // SM columns are now 1-based and locstrFull handles that, so we don't
          // subtract 1 off the length here.
          adjustedEnd.column = this._lines[adjustedEnd.line - 1].length;
        }
        obj.nestingRange = locstrFull(nestLoc.start, adjustedEnd);
      }
    }
    print(JSON.stringify(obj));
  },

  target(loc, name, kind, pretty, sym) {
    let locProp;
    if (typeof(loc) == "object" && "start" in loc) {
      locProp = locstr2(loc, name);
    } else {
      locProp = loc;
    }
    print(JSON.stringify({loc: locProp, target: 1, kind, pretty, sym,
                          context: this.context}));
  },

  defProp(name, loc, extra, extraPretty, maybeNesting) {
    if (!nameValid(name)) {
      return;
    }
    this.source(loc, name, "def,prop", `property ${name}`, `#${name}`, false,
                maybeNesting);
    this.target(loc, name, "def", name, `#${name}`);
    if (extra) {
      this.source(loc, name, "def,prop", `property ${extraPretty}`, extra,
                  false, maybeNesting);
      this.target(loc, name, "def", extraPretty, extra);
    }
  },

  useProp(name, loc, extra, extraPretty) {
    if (!nameValid(name)) {
      return;
    }
    this.source(loc, name, "use,prop", `property ${name}`, `#${name}`, false);
    this.target(loc, name, "use", name, `#${name}`);
    if (extra) {
      this.source(loc, name, "use,prop", `property ${extraPretty}`, extra,
                  false);
      this.target(loc, name, "use", extraPretty, extra);
    }
  },

  assignProp(name, loc, extra, extraPretty, maybeNesting) {
    if (!nameValid(name)) {
      return;
    }
    this.source(loc, name, "use,prop", `property ${name}`, `#${name}`, false,
                maybeNesting);
    this.target(loc, name, "assign", name, `#${name}`);
    if (extra) {
      this.source(loc, name, "use,prop", `property ${extraPretty}`, extra,
                  false, maybeNesting);
      this.target(loc, name, "assign", extraPretty, extra);
    }
  },

  defVar(name, loc, maybeNesting) {
    if (!nameValid(name)) {
      return;
    }
    if (this.isToplevel()) {
      this.defProp(name, loc, undefined, undefined, maybeNesting);
      return;
    }
    let sym = new SymbolTable.Symbol(name, loc);
    this.symbols.put(name, sym);

    this.source(loc, name, "deflocal,variable", `variable ${name}`, sym.id, true,
                maybeNesting);
  },

  findSymbol(name) {
    let sym = this.symbols.get(name);
    if (!sym) {
      for (let i = this.symbolTableStack.length - 1; i >= 0; i--) {
        sym = this.symbolTableStack[i].get(name);
        if (sym) {
          break;
        }
      }
    }
    return sym;
  },

  useVar(name, loc) {
    if (!nameValid(name)) {
      return;
    }
    let sym = this.findSymbol(name);
    if (!sym) {
      this.useProp(name, loc);
    } else if (!sym.skip) {
      this.source(loc, name, "uselocal,variable", `variable ${name}`, sym.id, true);
    }
  },

  assignVar(name, loc) {
    if (!nameValid(name)) {
      return;
    }
    let sym = this.findSymbol(name);
    if (!sym) {
      this.assignProp(name, loc);
    } else if (!sym.skip) {
      this.source(loc, name, "uselocal,variable", `variable ${name}`, sym.id, true);
    }
  },

  functionDecl(f) {
    for (let i = 0; i < f.params.length; i++) {
      this.pattern(f.params[i]);
      this.maybeExpression(f.defaults[i]);
    }
    if (f.rest) {
      this.defVar(f.rest.name, f.rest.loc);
    }
    if (f.body.type == "BlockStatement") {
      this.statement(f.body);
    } else {
      this.expression(f.body);
    }
  },

  statement(stmt) {
    switch (stmt.type) {
    case "EmptyStatement":
    case "BreakStatement":
    case "ContinueStatement":
    case "DebuggerStatement":
      break;

    case "BlockStatement":
      this.scoped(null, () => {
        for (let stmt2 of stmt.body) {
          this.statement(stmt2);
        }
      });
      break;

    case "ExpressionStatement":
      this.expression(stmt.expression);
      break;

    case "IfStatement":
      this.expression(stmt.test);
      this.statement(stmt.consequent);
      this.maybeStatement(stmt.alternate);
      break;

    case "LabeledStatement":
      this.statement(stmt.body);
      break;

    case "WithStatement":
      this.expression(stmt.object);
      this.statement(stmt.body);
      break;

    case "SwitchStatement":
      this.expression(stmt.discriminant);
      for (let scase of stmt.cases) {
        this.switchCase(scase);
      }
      break;

    case "ReturnStatement":
      this.maybeExpression(stmt.argument);
      break;

    case "ThrowStatement":
      this.expression(stmt.argument);
      break;

    case "TryStatement":
      this.statement(stmt.block);
      if (stmt.handler) {
        this.catchClause(stmt.handler);
      }
      this.maybeStatement(stmt.finalizer);
      break;

    case "WhileStatement":
      this.expression(stmt.test);
      this.statement(stmt.body);
      break;

    case "DoWhileStatement":
      this.statement(stmt.body);
      this.expression(stmt.test);
      break;

    case "ForStatement":
      this.scoped(null, () => {
        if (stmt.init && stmt.init.type == "VariableDeclaration") {
          this.variableDeclaration(stmt.init);
        } else if (stmt.init) {
          this.expression(stmt.init);
        }
        this.maybeExpression(stmt.test);
        this.maybeExpression(stmt.update);
        this.statement(stmt.body);
      });
      break;

    case "ForInStatement":
    case "ForOfStatement":
      this.scoped(null, () => {
        if (stmt.left && stmt.left.type == "VariableDeclaration") {
          this.variableDeclaration(stmt.left);
        } else {
          this.expression(stmt.left);
        }
        this.expression(stmt.right);
        this.statement(stmt.body);
      });
      break;

    case "LetStatement":
      this.scoped(null, () => {
        for (let decl of stmt.head) {
          this.variableDeclarator(decl);
        }
        this.statement(stmt.body);
      });
      break;

    case "FunctionDeclaration":
      this.defVar(stmt.id.name, stmt.loc, stmt.body);
      this.scoped(stmt.id.name, () => {
        this.functionDecl(stmt);
      });
      break;

    case "VariableDeclaration":
      this.variableDeclaration(stmt);
      break;

    //
    case "ClassStatement":
      this.defVar(stmt.id.name, stmt.id.loc,
                  this.deriveLocationFromOuterNodeAndIdNode(stmt, stmt.id));
      this.scoped(stmt.id.name, () => {
        let oldClass = this.className;
        this.className = stmt.id.name;
        if (stmt.superClass) {
          this.expression(stmt.superClass);
        }
        for (let stmt2 of stmt.body) {
          this.statement(stmt2);
        }
        this.className = oldClass;
      });
      break;

    case "ClassMethod": {
      let name = null;
      if (stmt.name.type == "Identifier") {
        name = stmt.name.name;
        this.defProp(
          stmt.name.name, stmt.name.loc,
          `${this.className}#${name}`, `${this.className}.${name}`,
          stmt.body);
      }

      this.scoped(name, () => {
        if (stmt.body.type == "FunctionExpression") {
          // Don't want to find the name twice.
          this.functionDecl(stmt.body);
        } else {
          this.expression(stmt.body);
        }
      });
      break;
    }

    // Class fields: https://github.com/tc39/proposal-class-fields
    // These are defined to have Object.defineProperty semantics.  The spec also
    // introduces private fields and these are partially supported, but
    // bug 1559269 disabled TokenStream support for them, so we don't support
    // them for now.
    case "ClassField": {
      let name = null;
      // name could be a computed name!
      if (stmt.name.type == "Identifier") {
        name = stmt.name.name;
        this.defProp(
          stmt.name.name, stmt.name.loc,
          `${this.className}#${name}`, `${this.className}.${name}`);
      }
      this.contextStack.push(name);
      if (stmt.init) {
        this.expression(stmt.init);
      }
      this.contextStack.pop();
      break;
    }

    case "StaticClassBlock": {
      this.statement(stmt.body);
      break;
    }

    case "ImportDeclaration": {
      for (const spec of stmt.specifiers) {
        if (spec.type === "ImportSpecifier" ||
            spec.type === "ImportNamespaceSpecifier") {
          this.pattern(spec.name);

          if (spec.type === "ImportSpecifier" &&
              !isSameLocation(spec.id.loc, spec.name.loc)) {
            this.expression(spec.id);
          }
        }
      }

      if (stmt.moduleRequest && stmt.moduleRequest.source &&
          stmt.moduleRequest.source.type === "Literal") {
        this.maybeLinkifyLiteral(stmt.moduleRequest.source);
      }
      break;
    }

    case "ExportDeclaration": {
      if (stmt.declaration) {
        if (stmt.declaration.type === "FunctionDeclaration") {
          if (stmt.declaration.id) {
            this.statement(stmt.declaration);
          }
        }
        else if (stmt.declaration.type === "VariableDeclaration" ||
                 stmt.declaration.type === "ClassStatement") {
          this.statement(stmt.declaration);
        } else {
          this.expression(stmt.declaration);
        }
      }

      if (stmt.specifiers) {
        for (const spec of stmt.specifiers) {
          if (spec.type === "ExportSpecifier" ||
              spec.type === "ExportNamespaceSpecifier") {
            if (spec.name.type !== "Literal") {
              this.pattern(spec.name);
            }

            if (spec.type === "ExportSpecifier" &&
                !isSameLocation(spec.id.loc, spec.name.loc)) {
              this.expression(spec.id);
            }
          }
        }
      }

      if (stmt.moduleRequest && stmt.moduleRequest.source &&
          stmt.moduleRequest.source.type === "Literal") {
        this.maybeLinkifyLiteral(stmt.moduleRequest.source);
      }
      break;
    }

    default:
      throw "Unexpected statement: " + stmt.type + " " + JSON.stringify(stmt);
      break;
    }
  },

  variableDeclaration(decl) {
    for (let d of decl.declarations) {
      this.variableDeclarator(d);
    }
  },

  variableDeclarator(decl) {
    this.pattern(decl.id);

    let oldNameForThis = this.nameForThis;
    if (decl.id.type == "Identifier" && decl.init) {
      if (decl.init.type == "ObjectExpression") {
        this.nameForThis = decl.id.name;
      } else {
        // Handle Object.freeze({...})
      }
    }
    this.contextStack.push(this.nameForThis);
    this.maybeExpression(decl.init);
    this.contextStack.pop();
    this.nameForThis = oldNameForThis;
  },

  maybeStatement(stmt) {
    if (stmt) {
      this.statement(stmt);
    }
  },

  maybeExpression(expr) {
    if (expr) {
      this.expression(expr);
    }
  },

  switchCase(scase) {
    if (scase.test) {
      this.expression(scase.test);
    }
    for (let stmt of scase.consequent) {
      this.statement(stmt);
    }
  },

  catchClause(clause) {
    if (clause.param) {
      this.pattern(clause.param);
    }
    if (clause.guard) {
      this.expression(clause.guard);
    }
    this.statement(clause.body);
  },

  expression(expr) {
    if (!expr) print(Error().stack);

    switch (expr.type) {
    case "Identifier":
      this.useVar(expr.name, expr.loc);
      break;

    case "Literal":
      this.maybeLinkifyLiteral(expr);
      break;

    case "Super":
      break;

    case "TemplateLiteral":
      for (let elt of expr.elements) {
        this.expression(elt);
      }
      break;

    case "TaggedTemplate":
      // Do something eventually!
      break;

    case "ThisExpression":
      // Do something eventually!
      break;

    case "ArrayExpression":
    case "ArrayPattern":
      for (let elt of expr.elements) {
        this.maybeExpression(elt);
      }
      break;

    case "ObjectExpression":
    case "ObjectPattern":
      for (let prop of expr.properties) {
        if (prop.type === "SpreadExpression") {
          this.expression(prop.expression);
          continue;
        }

        let name;

        if (prop.key) {
          let loc;
          if (prop.key.type == "Identifier") {
            name = prop.key.name;
            loc = prop.key.loc;
          } else if (prop.key.type == "Literal" && typeof(prop.key.value) == "string") {
            name = prop.key.value;
            loc = prop.key.loc;
            loc.start.column++;
          }
          let extra = null;
          let extraPretty = null;
          if (this.nameForThis) {
            extra = `${this.nameForThis}#${name}`;
            extraPretty = `${this.nameForThis}.${name}`;
          }
          if (name) {
            this.defProp(name, prop.key.loc, extra, extraPretty, prop.value);
          }
        }

        this.contextStack.push(name);
        if (prop.value) {
          this.expression(prop.value);
        }
        this.contextStack.pop();
      }
      break;

    case "FunctionExpression":
    case "ArrowFunctionExpression":
      // In theory this could declare a variable that can be used in
      // the function. But most of the time, it appears on class
      // methods that don't actually define such a variable. This is
      // probably a SpiderMonkey bug. We just don't do anything here
      // to be correct in the common case.
      //let name = expr.id ? expr.id.name : "";
      let name = null;
      this.scoped(name, () => {
        if (this.className && name == this.className) {
          // SPIDERMONKEY HACK: Fixes a bug where constructors get the
          // name of their class instead of "constructor".
          name = "constructor";
        }

        if (expr.type == "FunctionExpression" && name) {
          this.defVar(name, expr.loc);
        }

        this.functionDecl(expr);
      });
      break;

    case "SequenceExpression":
      for (let elt of expr.expressions) {
        this.expression(elt);
      }
      break;

    case "UnaryExpression":
    case "UpdateExpression":
      this.expression(expr.argument);
      break;

    case "AssignmentExpression":
      if (expr.left.type == "Identifier") {
        this.assignVar(expr.left.name, expr.left.loc);
      } else if (expr.left.type == "MemberExpression" && !expr.left.computed) {
        this.expression(expr.left.object);

        let extra = null;
        let extraPretty = null;
        if (expr.left.object.type == "ThisExpression" && this.nameForThis) {
          extra = `${this.nameForThis}#${expr.left.property.name}`;
          extraPretty = `${this.nameForThis}.${expr.left.property.name}`;
        } else if (expr.left.object.type == "Identifier") {
          extra = `${expr.left.object.name}#${expr.left.property.name}`;
          extraPretty = `${expr.left.object.name}.${expr.left.property.name}`;
        }
        this.assignProp(expr.left.property.name, memberPropLoc(expr.left), extra, extraPretty,
                        expr.right.loc);
      } else {
        this.expression(expr.left);
      }

      let oldNameForThis = this.nameForThis;
      if (expr.left.type == "MemberExpression" &&
          !expr.left.computed)
      {
        if (expr.left.property.name == "prototype" &&
            expr.left.object.type == "Identifier")
        {
          this.nameForThis = expr.left.object.name;
        }
        if (expr.left.object.type == "ThisExpression") {
          this.nameForThis = expr.left.property.name;
        }
      }
      this.contextStack.push(this.nameForThis);
      this.expression(expr.right);
      this.contextStack.pop();
      this.nameForThis = oldNameForThis;
      break;

    case "BinaryExpression":
    case "LogicalExpression":
      this.expression(expr.left);
      this.expression(expr.right);
      break;

    case "ConditionalExpression":
      this.expression(expr.test);
      this.expression(expr.consequent);
      this.expression(expr.alternate);
      break;

    case "NewExpression":
    case "CallExpression":
    case "OptionalCallExpression":
      this.expression(expr.callee);
      for (let arg of expr.arguments) {
        this.expression(arg);
      }
      break;

    case "MemberExpression":
    case "OptionalMemberExpression":
      this.expression(expr.object);
      if (expr.computed) {
        this.expression(expr.property);
      } else {
        let extra = null;
        let extraPretty = null;
        if (expr.object.type == "ThisExpression" && this.nameForThis) {
          extra = `${this.nameForThis}#${expr.property.name}`;
          extraPretty = `${this.nameForThis}.${expr.property.name}`;
        } else if (expr.object.type == "Identifier") {
          extra = `${expr.object.name}#${expr.property.name}`;
          extraPretty = `${expr.object.name}.${expr.property.name}`;
        }

        this.useProp(expr.property.name, memberPropLoc(expr), extra, extraPretty);
      }
      break;

    case "YieldExpression":
      this.maybeExpression(expr.argument);
      break;

    case "SpreadExpression":
      this.expression(expr.expression);
      break;

    case "ComprehensionExpression":
    case "GeneratorExpression":
      this.scoped(null, () => {
        let before = locBefore(expr.body.loc, expr.blocks[0].loc);
        if (before) {
          this.expression(expr.body);
        }
        for (let block of expr.blocks) {
          this.comprehensionBlock(block);
        }
        this.maybeExpression(expr.filter);
        if (!before) {
          this.expression(expr.body);
        }
      });
      break;

    case "ClassExpression":
      this.scoped(null, () => {
        if (expr.superClass) {
          this.expression(expr.superClass);
        }
        for (let stmt2 of expr.body) {
          this.statement(stmt2);
        }
      });
      break;

    case "OptionalExpression":
    case "DeleteOptionalExpression":
      // a?.b is an optional expression that is equivalent to a && a.b.
      // expr.expression is an OptionalMemberExpression or OptionalCallExpression
      this.expression(expr.expression);
      break;

    case "MetaProperty": // Not sure what this is!
    case "CallImport": // dynamic import statement, see e.g. https://hg.mozilla.org/mozilla-central/file/4df1ba9c741f/testing/web-platform/tests/html/semantics/scripting-1/the-script-element/module/dynamic-import/propagate-nonce-external.js#l3
      break;

    default:
      printErr(Error().stack);
      throw `Invalid expression ${expr.type}: ${JSON.stringify(expr)}`;
      break;
    }
  },

  comprehensionBlock(block) {
    switch (block.type) {
    case "ComprehensionBlock":
      this.pattern(block.left);
      this.expression(block.right);
      break;

    case "ComprehensionIf":
      this.expression(block.test);
      break;
    }
  },

  pattern(pat) {
    if (!pat) {
      print(Error().stack);
    }

    switch (pat.type) {
    case "Identifier":
      this.defVar(pat.name, pat.loc);
      break;

    case "ObjectPattern":
      for (let prop of pat.properties) {
        if (prop.type == "Property") {
          this.pattern(prop.value);
        } else if (prop.type == "SpreadExpression") {
          this.pattern(prop.expression);
        } else {
          throw `Unexpected prop ${JSON.stringify(prop)} in ObjectPattern`;
        }
      }
      break;

    case "ArrayPattern":
      for (let e of pat.elements) {
        if (e) {
          this.pattern(e);
        }
      }
      break;

    case "SpreadExpression":
      this.pattern(pat.expression);
      break;

    case "AssignmentExpression":
      this.pattern(pat.left);
      this.expression(pat.right);
      break;

    default:
      throw `Unexpected pattern: ${pat.type} ${JSON.stringify(pat)}`;
      break;
    }
  },

  maybeLinkifyLiteral(expr) {
    if (typeof expr.value !== "string") {
      return;
    }

    if (!expr.value.startsWith("chrome://") &&
        !expr.value.startsWith("resource://")) {
      return;
    }

    const name = "\"" + expr.value + "\"";
    const loc = expr.loc;
    const url = expr.value;
    const sym = "URL_" + atEscape(url);
    this.source(loc, name, "file,use", "type " + url, sym);
    this.target(loc, name, "use", url, sym);
  },
};

function printFileTarget(path) {
  print(JSON.stringify({
    loc: "00001:0",
    target: 1,
    kind: "def",
    pretty: "file " + path,
    sym: "FILE_" + atEscape(path),
  }));
}

// Helper for preprocessor directives so that JS assignments like `#error =`
// won't match.  All of this is obviously optimized for clarity/not messing up
// regexps, as we could combine most of the preproccesing checks into very few
// super-regexps.
function startsWithNoEquals(subjectString, checkString) {
  if (!subjectString.startsWith(checkString)) {
    return false;
  }
  if (subjectString.substring(checkString.length).trimStart()[0] === "=") {
    return false;
  }
  return true;
}

function preprocess(filename, comment)
{
  // Set the filename so that logError can downgrade any errors/warnings to INFO
  // if the filename has the word "error" in it.
  gFilename = filename;
  gIncludeUsed = false;
  gCouldBeJson = false;

  let text;
  try {
    text = snarf(filename);

    // There are a few `.js` files in the tree that use `#` as a comment for a
    // preprocessed file for the MPL and this is not helpful.  One is also a
    // mozconfig.  Just no-op the file.
    // https://searchfox.org/mozilla-central/search?q=path%3A.js%20%23%20This%20Source%20Code%20Form%20is%20subject%20to%20the%20terms%20of%20the%20Mozilla%20Public&path=
    // okay, also l10n (which is also getting a file constraint"):
    // https://searchfox.org/l10n/source/tn/mail/all-l10n.js
    if (text.startsWith("# This Source Code Form is subject to the terms of the Mozilla Public") ||
        text.startsWith("# ***** BEGIN LICENSE BLOCK *****")) {
      text = "";
    }
  } catch (e) {
    text = "";
  }

  if (text.startsWith("{")) {
    gCouldBeJson = true;
  }

  let substitution = false;
  let lines = text.split("\n");
  let preprocessedLines = [];
  let branches = [true];
  for (let i = 0; i < lines.length; i++) {
    let line = lines[i];
    if (substitution) {
      line = line.replace(/@(\w+)@/, "''");
    }
    let tline = line.trim();
    if (startsWithNoEquals(tline, "#ifdef ") || startsWithNoEquals(tline, "#ifndef ") || startsWithNoEquals(tline, "#if ")) {
      preprocessedLines.push(comment(tline));
      branches.push(branches[branches.length-1]);
    } else if (tline.startsWith("#else") ||
               startsWithNoEquals(tline, "#elif ") ||
               startsWithNoEquals(tline, "#elifdef ") ||
               startsWithNoEquals(tline, "#elifndef ")) {
      preprocessedLines.push(comment(tline));
      branches.pop();
      branches.push(false);
    } else if (tline.startsWith("#endif")) {
      preprocessedLines.push(comment(tline));
      branches.pop();
    } else if (!branches[branches.length-1]) {
      preprocessedLines.push(comment(tline));
    } else if (startsWithNoEquals(tline, "#include ") || startsWithNoEquals(tline, "#includesubst ")) {
      // Mark that we used an include so we know this file may experience parse
      // errors which should be downgraded to INFO from WARN.
      gIncludeUsed = true;

      /*
      let match = tline.match(/#include "?([A-Za-z0-9_.-]+)"?/);
      if (!match) {
        throw new Error(`Invalid include directive: ${filename}:${i+1}`);
      }
      let incfile = match[1];
      preprocessedLines.push(`PREPROCESSOR_INCLUDE("${incfile}");`);
      */
      preprocessedLines.push(comment(tline));
    } else if (tline.startsWith("#filter substitution")) {
      preprocessedLines.push(comment(tline));
      substitution = true;
      // require whitespace after the filter to avoid catching variable names
      // like `#filterLogins`.
    } else if (startsWithNoEquals(tline, "#filter ") || startsWithNoEquals(tline, "#unfilter ")) {
      preprocessedLines.push(comment(tline));
    } else if (startsWithNoEquals(tline, "#expand ")) {
      preprocessedLines.push(line.substring(String("#expand ").length));
    } else if (startsWithNoEquals(tline, "#literal ")) {
        preprocessedLines.push(line.substring(String("#literal ").length));
    } else if (startsWithNoEquals(tline, "#define ") ||
               startsWithNoEquals(tline, "#undef ") ||
               startsWithNoEquals(tline, "#error ")) {
      preprocessedLines.push(comment(tline));
    } else {
      preprocessedLines.push(line);
    }
  }

  return preprocessedLines.join("\n");
}

function analyzeJS(filename)
{
  let text = preprocess(filename, line => "// " + line);

  let target = filename.endsWith(".mjs") ? "module" : "script";

  let ast = Analyzer.parse(text, filename, 1, target);
  if (ast) {
    try {
      Analyzer.program(ast);
    } catch (ex) {
      logError(`In ${filename}, got: ${ex}`);
    }
  }
}

function replaceEntities(text)
{
  var table = {
    "&amp;&amp;": "&&        ",
    "&amp;": "&    ",
    "&lt;": "<   ",
    "&gt;": ">   ",
  };

  for (let ent in table) {
    let re = RegExp(ent, "gi");
    text = text.replace(re, table[ent]);
  }

  return text;
}

// XXX SpiderMonkey now uses 1-based column numbers since bug 1862692.  The SAX
// parser is definitely 0-based but I think we just create JS strings that get
// parsed into the JS universe so it doesn't matter that this parser is using
// 0-based columns.  But maybe I'm wrong?  I'm doing a quick fix.  Also, we
// don't really have XUL files anymore...
class BaseParser {
  constructor(filename, parser) {
    this.filename = filename;
    this.stack = [];
    this.curAttrs = {};
    this.parser = parser;
    this.eventListeners = [];

    for (let prop of ["onopentag", "onclosetag", "onattribute"]) {
      parser[prop] = this[prop].bind(this);
    }
  }

  onopentag(tag) {
    tag.line = this.parser.line;
    tag.column = this.parser.column;
    tag.attrs = this.curAttrs;
    this.curAttrs = {};
    this.stack.push(tag);
  }

  onclosetag(tagName) {
    let tag = this.stack[this.stack.length - 1];

    this.ontag(tagName, tag);

    this.stack.pop();
  }

  ontag(tagName, tag) {
  }

  onattribute(attr) {
    this.curAttrs[attr.name] = attr;
  }

  handleAttributes(tag) {
    for (let prop in tag.attrs) {
      if (prop.startsWith("ON")) {
        this.handleEventListener(tag, prop);
        continue;
      }
      if (prop == "STYLE") {
        this.handleStyleProp(tag, prop);
        continue;
      }

      let text = tag.attrs[prop].value;
      if (text.startsWith("chrome://") || text.startsWith("resource://")) {
        this.handleURLAttribute(tag, prop);
      }
    }
  }

  handleEventListener(tag, prop) {
    let text = tag.attrs[prop].value;
    let line = tag.attrs[prop].valueLine;
    let column = tag.attrs[prop].valueColumn;

    let spaces = " ".repeat(column);
    text = `(function (val) {\n${spaces}${text}\n})`;

    let ast = Analyzer.parse(text, this.filename, line, "script", prop);
    if (ast) {
      this.eventListeners.push(ast);
    }
  }

  processEventListeners() {
    for (let ast of this.eventListeners) {
      Analyzer.dummyProgram(ast, [{name: "event", skip: true}]);
    }
  }

  handleURLAttribute(tag, prop) {
    let url = tag.attrs[prop].value;
    let line = tag.attrs[prop].valueLine;
    let column = tag.attrs[prop].valueColumn;

    const locStr = `${line + 1}:${column}-${column + url.length}`;
    const sym = "URL_" + atEscape(url);
    Analyzer.source(locStr, url, "file,use", "type " + url, sym);
    Analyzer.target(locStr, url, "use", url, sym);
  }

  getScriptTarget(tag) {
    let type;
    if ("TYPE" in tag.attrs) {
      type = tag.attrs.TYPE.value;
    } else if ("LANGUAGE" in tag.attrs) {
      type = "text/" + tag.attrs.LANGUAGE.value;
    } else {
      return "script";
    }
    if (type === "module") {
      return "module";
    }
    const jsMIMETypes = [
      "text/javascript",
      "text/ecmascript",
      "application/javascript",
      "application/ecmascript",
      "application/x-javascript",
      "application/x-ecmascript",
      "text/javascript1.0",
      "text/javascript1.1",
      "text/javascript1.2",
      "text/javascript1.3",
      "text/javascript1.4",
      "text/javascript1.5",
      "text/jscript",
      "text/livescript",
      "text/x-ecmascript",
      "text/x-javascript",
    ];
    if (jsMIMETypes.includes(type.toLowerCase())) {
      return "script";
    }
    return "";
  }

  handleScript(text, tag) {
    let target = this.getScriptTarget(tag);

    if (target !== "script" && target !== "module") {
      return;
    }

    let {line, column} = tag;

    let spaces = " ".repeat(column);
    text = spaces + text;

    let ast = Analyzer.parse(text, this.filename, line + 1, target);
    if (ast) {
      Analyzer.program(ast);
    }
  }

  handleStyle(text, tag) {
    let {line, column} = tag;

    let spaces = " ".repeat(column);
    text = spaces + text;

    const analyzer = new CSSAnalyzer({ line: line + 1 });
    analyzer.parse(text);
  }

  handleStyleProp(tag, prop) {
    let text = tag.attrs[prop].value;
    let line = tag.attrs[prop].valueLine;
    let column = tag.attrs[prop].valueColumn;

    let spaces = " ".repeat(column);
    text = spaces + text;

    const analyzer = new CSSAnalyzer({ line: line + 1 });
    analyzer.parse(text);
  }
}

class XMLParser extends BaseParser {
  constructor(filename, parser) {
    super(filename, parser)
    this.curText = "";
    for (let prop of ["ontext", "oncdata"]) {
      parser[prop] = this[prop].bind(this);
    }
  }

  onopentag(tag) {
    super.onopentag(tag);
    this.curText = "";
  }

  ontext(text) {
    this.curText += text;
  }

  oncdata(text) {
    this.curText += replaceEntities(text);
  }
}

class XBLParser extends XMLParser {
  ontag(tagName, tag) {
    switch (tagName) {
    case "FIELD":
      this.onfield(tag);
      break;
    case "PROPERTY":
      this.onproperty(tag);
      break;
    case "GETTER":
      this.ongetter(tag);
      break;
    case "SETTER":
      this.onsetter(tag);
      break;
    case "METHOD":
      this.onmethod(tag);
      break;
    case "PARAMETER":
      this.onparameter(tag);
      break;
    case "BODY":
      this.onbody(tag);
      break;
    case "CONSTRUCTOR":
    case "DESTRUCTOR":
      this.onstructor(tag);
      break;
    case "HANDLER":
      this.onhandler(tag);
      break;
    }
  }

  onfield(tag) {
    if (!tag.attrs.NAME) {
      return;
    }

    let line = tag.attrs.NAME.valueLine;
    let column = tag.attrs.NAME.valueColumn;
    let name = tag.attrs.NAME.value;

    let locStr = `${line + 1}:${column}-${column + name.length}`;
    Analyzer.source(locStr, name, "def,prop", `property ${name}`, `#${name}`,
                    false);
    Analyzer.target(locStr, name, "def", name, `#${name}`);

    let spaces = " ".repeat(tag.column);
    let text = spaces + this.curText;

    let ast = Analyzer.parse(text, this.filename, tag.line + 1, "script");
    if (ast) {
      Analyzer.program(ast);
    }
  }

  onproperty(tag) {
    let name = null;
    if (tag.attrs.NAME) {
      let line = tag.attrs.NAME.valueLine;
      let column = tag.attrs.NAME.valueColumn;
      name = tag.attrs.NAME.value;

      let locStr = `${line + 1}:${column}-${column + name.length}`;
      Analyzer.source(locStr, name, "def,prop", `property ${name}`, `#${name}`,
                      false);
      Analyzer.target(locStr, name, "def", name, `#${name}`);
    }

    let line, column;
    for (let prop in tag.attrs) {
      if (prop != "ONGET" && prop != "ONSET") {
        continue;
      }

      let text = tag.attrs[prop].value;
      line = tag.attrs[prop].valueLine;
      column = tag.attrs[prop].valueColumn;

      let spaces = " ".repeat(column);
      text = `(function (val) {\n${spaces}${text}\n})`;

      let ast = Analyzer.parse(text, this.filename, line, "script", prop);
      if (ast) {
        Analyzer.scoped(name, () => Analyzer.dummyProgram(ast, [{name: "val", skip: true}]));
      }
    }

    for (let prop in tag) {
      if (prop != "getter" && prop != "setter") {
        continue;
      }

      let text = tag[prop].text;
      line = tag[prop].line;
      column = tag[prop].column;

      let spaces = " ".repeat(column);
      text = `(function (val) {\n${spaces}${text}\n})`;

      let ast = Analyzer.parse(text, this.filename, line, "script", prop);
      if (ast) {
        Analyzer.scoped(name, () => Analyzer.dummyProgram(ast, [{name: "val", skip: true}]));
      }
    }
  }

  ongetter(tag) {
    tag.text = this.curText;
    let parentTag = this.stack[this.stack.length - 2];
    if (parentTag) {
      parentTag.getter = tag;
    }
  }

  onsetter(tag) {
    tag.text = this.curText;
    let parentTag = this.stack[this.stack.length - 2];
    if (parentTag) {
      parentTag.setter = tag;
    }
  }

  onparameter(tag) {
    let parentTag = this.stack[this.stack.length - 2];
    if (parentTag) {
      if (!parentTag.params) {
        parentTag.params = [];
      }
      parentTag.params.push(tag);
    }
  }

  onbody(tag) {
    tag.text = this.curText;
    let parentTag = this.stack[this.stack.length - 2];
    if (parentTag) {
      parentTag.body = tag;
    }
  }

  onstructor(tag) {
    let text = this.curText;
    let {line, column} = tag;

    let spaces = " ".repeat(column);
    text = `(function () {\n${spaces}${text}\n})`;

    let ast = Analyzer.parse(text, this.filename, line, "script");
    if (ast) {
      Analyzer.scoped(null, () => Analyzer.dummyProgram(ast, []));
    }
  }

  onhandler(tag) {
    let text = this.curText;
    let {line, column} = tag;

    let spaces = " ".repeat(column);
    text = `(function () {\n${spaces}${text}\n})`;

    let ast = Analyzer.parse(text, this.filename, line, "script");
    if (ast) {
      Analyzer.scoped(null, () => Analyzer.dummyProgram(ast, []));
    }
  }

  onmethod(tag) {
    if (!tag.attrs.NAME) {
      return;
    }

    let line = tag.attrs.NAME.valueLine;
    let column = tag.attrs.NAME.valueColumn;
    let name = tag.attrs.NAME.value;

    let locStr = `${line + 1}:${column}-${column + name.length}`;
    Analyzer.source(locStr, name, "def,prop", `property ${name}`, `#${name}`,
                    false);
    Analyzer.target(locStr, name, "def", name, `#${name}`);

    Analyzer.enter(name);

    let params = tag.params || [];
    for (let p of params) {
      let text = p.attrs.NAME.value;
      line = p.attrs.NAME.valueLine;
      column = p.attrs.NAME.valueColumn;

      Analyzer.defVar(text, {start: {line: line + 1, column}});
    }

    if (tag.body) {
      let text = tag.body.text;
      line = tag.body.line;
      column = tag.body.column;

      params = params.map(p => p.attrs.NAME.value);
      let paramsText = params.join(", ");

      let spaces = " ".repeat(column);
      text = `(function (${paramsText}) {\n${spaces}${text}\n})`;

      let ast = Analyzer.parse(text, this.filename, line, "script");
      if (ast) {
        Analyzer.dummyProgram(ast, []);
      }
    }

    Analyzer.exit();
  }
}

// Prepare the `sax` global variable.
// This function shouldn't be called multiple times.
//
// js-analyze.js is executed for single file, and the code path
// "analyzeFile -> analyze* -> loadSax" is taken at most once.
function loadSax()
{
  load(mozSearchRoot + "/sax/sax.js");
}

function analyzeXBL(filename)
{
  let text = preprocess(filename, line => `<!--${line}-->`);

  loadSax();
  let parser = sax.parser(false, {trim: false, normalize: false, xmlns: true, position: true});

  new XBLParser(filename, parser);

  parser.write(text);
  parser.close();
}

class XULParser extends XMLParser {
  ontag(tagName, tag) {
    switch (tagName) {
    case "SCRIPT":
      this.handleScript(this.curText, tag);
      break;
    }

    this.handleAttributes(tag);
  }
}

class HTMLParser extends BaseParser {
  constructor(filename, parser) {
    super(filename, parser);

    this.inStyle = false;
    this.currentStyle = "";
    for (let prop of ["onscript", "ontext"]) {
      parser[prop] = this[prop].bind(this);
    }
  }

  onopentag(tag) {
    super.onopentag(tag);

    if (tag.local.toUpperCase() === "STYLE") {
      this.inStyle = true;
      this.currentStyle = "";
    }
  }

  ontext(text) {
    if (this.inStyle) {
      this.currentStyle += text;
    }
  }

  ontag(tagName, tag) {
    switch (tagName) {
    case "SCRIPT":
      this.handleScript(this.currentScript, tag);
      break;
    case "STYLE":
      this.inStyle = false;
      this.handleStyle(this.currentStyle, tag);
      break;
    }

    this.handleAttributes(tag);
  }

  onscript(script) {
    this.currentScript = replaceEntities(script);
  }
}

function analyzeXUL(filename)
{
  let text = preprocess(filename, line => `<!--${line}-->`);

  if (filename.endsWith(".inc")) {
    text = "<root>" + text + "</root>";
  }

  loadSax();
  let parser = sax.parser(false, {trim: false, normalize: false, xmlns: true, position: true, noscript: true});

  let parser2 = new XULParser(filename, parser);

  parser.write(text);
  parser.close();

  parser2.processEventListeners();
}

function analyzeHTML(filename)
{
  let text = preprocess(filename, line => `<!--${line}-->`);

  if (filename.endsWith(".inc")) {
    text = "<root>" + text + "</root>";
  }

  loadSax();
  let parser = sax.parser(false, {trim: false, normalize: false, xmlns: true, position: true, noscript: false});

  let parser2 = new HTMLParser(filename, parser);

  parser.write(text);
  parser.close();

  parser2.processEventListeners();
}

class CSSAnalyzer {
  static analyze_css_source = null;

  static ensureCSSAnalyzer() {
    if (CSSAnalyzer.analyze_css_source) {
      return;
    }

    const wasmPath = mozSearchRoot + "/scripts/web-analyze/wasm-css-analyzer/out";
    const wasmBinary = createMappedArrayBuffer(wasmPath + "/wasm_css_analyzer.wasm");

    // getrandom crate requires WebCrypto API.
    const MyCrypto = {
      getRandomValues(array) {
        let i = 0, length = array.length;
        while (i < length) {
          array[i++] = Math.random() * 256;
        }
        return array;
      }
    };

    // The binding JS requires TextEncoder and TextDecoder.
    class MyTextEncoder {
      encode(text) {
        // This is called only when the text is non-ASCII.

        let units = [], index = 0, length = text.length,
            n, trail, b1, b2, b3, b4;

        const NonBMPMin = 0x10000,
              NonBMPMax = 0x10FFFF,
              LeadSurrogateMin = 0xD800,
              LeadSurrogateMax = 0xDBFF,
              TrailSurrogateMin = 0xDC00,
              TrailSurrogateMax = 0xDFFF;

        while (index < length) {
          n = text.charCodeAt(index++);

          if (n <= 0x7F) {
            units.push(n);
            continue;
          }

          if (n >= LeadSurrogateMin && n <= LeadSurrogateMax) {
            trail = text.charCodeAt(index++);
            n = (n << 10) + trail +
              (NonBMPMin - (LeadSurrogateMin << 10) - TrailSurrogateMin);
          }

          if (n > NonBMPMax) {
            units.push(0x3F);
          } else if (n >= 0x010000) {
            b4 = n & 0x3F;
            n >>= 6;
            b3 = n & 0x3F;
            n >>= 6;
            b2 = n & 0x3F;
            n >>= 6;
            b1 = n & 0x3F;
            units.push(b1 | 0b1111_0000);
            units.push(b2 | 0b1000_0000);
            units.push(b3 | 0b1000_0000);
            units.push(b4 | 0b1000_0000);
          } else if (n >= 0x0800) {
            b3 = n & 0x3F;
            n >>= 6;
            b2 = n & 0x3F;
            n >>= 6;
            b1 = n & 0x3F;
            units.push(b1 | 0b1110_0000);
            units.push(b2 | 0b1000_0000);
            units.push(b3 | 0b1000_0000);
          } else {
            b2 = n & 0x3F;
            n >>= 6;
            b1 = n & 0x3F;
            units.push(b1 | 0b1100_0000);
            units.push(b2 | 0b1000_0000);
          }
        }

        return new Uint8Array(units);
      }
    }

    class MyTextDecoder {
      decode(buffer) {
        // This is used for all string received from wasm.
        // This is called only with complete data.

        if (buffer === undefined) {
          return "";
        }

        // Converted from DecodeOneUtf8CodePointInline in m-c/mfbt/Utf8.h,
        // with substituting bad code units with "?".

        let chars = [], index = 0, length = buffer.length,
            n, remaining, min, actual, i, unit;

        next: while (index < length) {
          n = buffer[index++];

          if ((n & 0b1000_0000) == 0b0000_0000) {
            chars.push(String.fromCodePoint(n));
            continue;
          }

          // |n| determines the number of trailing code units in the code point
          // and the bits of |n| that contribute to the code point's value.
          if ((n & 0b1110_0000) == 0b1100_0000) {
            remaining = 1;
            min = 0x80;
            n &= 0b0001_1111;
          } else if ((n & 0b1111_0000) == 0b1110_0000) {
            remaining = 2;
            min = 0x800;
            n &= 0b0000_1111;
          } else if ((n & 0b1111_1000) == 0b1111_0000) {
            remaining = 3;
            min = 0x10000;
            n &= 0b0000_0111;
          } else {
            chars.push("?");
            continue;
          }

          // If the code point would require more code units than remain, the encoding
          // is invalid.
          actual = length - i;
          if (actual < remaining) {
            chars.push("?");
            continue;
          }

          for (i = 0; i < remaining; i++) {
            unit = buffer[index++];

            // Every non-leading code unit in properly encoded UTF-8 has its high
            // bit set and the next-highest bit unset.
            if (!((unit & 0b1100_0000) == 0b1000_0000)) {
              index -= i + 1;
              chars.push("?");
              continue next;
            }

            // The code point being encoded is the concatenation of all the
            // unconstrained bits.
            n = (n << 6) | (unit & 0b0011_1111);
          }

          // UTF-16 surrogates and values outside the Unicode range are invalid.
          if (n > 0x10FFFF || (0xD800 <= n && n <= 0xDFFF)) {
            index -= remaining;
            chars.push("?");
            continue;
          }

          // Overlong code points are also invalid.
          if (n < min) {
            index -= remaining;
            chars.push("?");
            continue;
          }

          chars.push(String.fromCodePoint(n));
        }

        return chars.join("");
      }
    }

    globalThis.crypto = MyCrypto;
    globalThis.TextEncoder = MyTextEncoder;
    globalThis.TextDecoder = MyTextDecoder;
    load(wasmPath + "/wasm_css_analyzer.js");
    globalThis.initSync(wasmBinary);

    CSSAnalyzer.analyze_css_source = globalThis.analyze_css_source;
  }

  constructor({ line = 1 } = {}) {
    CSSAnalyzer.ensureCSSAnalyzer();

    this.startLine = line;
  }

  parse(text) {
    CSSAnalyzer.analyze_css_source(text, this.startLine, function(s) {
      print(s);
    });
  }
}

function analyzeFile(filename)
{
  if (filename.endsWith(".xml")) {
    analyzeXBL(filename);
  } else if (filename.endsWith(".xul") || filename.endsWith(".inc") || filename.endsWith(".xhtml")) {
    analyzeXUL(filename);
  } else if (filename.endsWith(".html")) {
    analyzeHTML(filename);
  } else {
    analyzeJS(filename);
  }
}

fileIndex = scriptArgs[0];
mozSearchRoot = scriptArgs[1];
localFile = scriptArgs[2];
sourcePath = scriptArgs[3];

printFileTarget(sourcePath);

analyzeFile(localFile);
