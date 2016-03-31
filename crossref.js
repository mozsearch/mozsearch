let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let objdir = scriptArgs[3];
let filenamesFile = scriptArgs[4];

let analysisRoot = indexRoot + "/analysis";
let outputFile = indexRoot + "/crossref";
let jumpFile = indexRoot + "/jumps";

run(mozSearchRoot + "/lib.js");
run(mozSearchRoot + "/output.js");

let identifiers = new Map();
let pretty = new Map();

function cut(str, n)
{
  if (str.length > n) {
    return str.substring(0, n) + "...";
  } else {
    return str;
  }
}

function processFile(path)
{
  if (!path) {
    return;
  }

  let source = sourcePath(path);

  let code;
  try {
    code = snarf(source);
  } catch (e) {
    return;
  }
  let analysis = readAnalysis(analysisRoot + "/" + path, j => j.target);

  let codeLines = code.split("\n");

  function put(id, loc, kind) {
    if (!identifiers.has(id)) {
      identifiers.set(id, {});
    }
    let obj = identifiers.get(id);
    if (!(kind in obj)) {
      obj[kind] = new Map();
    }
    let files = obj[kind];
    if (!files.has(path)) {
      files.set(path, []);
    }
    let line = codeLines[loc.line - 1] || "";
    files.get(path).push({ lno: loc.line, line: cut(line.trim(), 100) });
  }

  for (let datum of analysis.targets) {
    let loc = datum.loc;
    for (let inner of datum.analysis) {
      put(inner.sym, loc, inner.kind);
      pretty.set(inner.sym, inner.pretty);
    }
  }
}

function writeMap()
{
  let jumps = new Map();

  function build(obj) {
    function buildKind(kind) {
      if (!obj[kind]) {
        return [];
      }
      let result = Array.from(obj[kind], ([path, lines]) => { return {path, lines}; });
      if (result.length > 1000) {
        return result.slice(0, 1000);
      } else {
        return result;
      }
    }

    return {
      "Uses": buildKind("use"),
      "Definitions": buildKind("def"),
      "Declarations": buildKind("decl"),
      "Assignments": buildKind("assign"),
      "IDL": buildKind("idl"),
    };
  }

  let old = redirect(outputFile);

  for (let [id, obj] of identifiers) {
    print(id);
    print(JSON.stringify(build(obj)));

    if (obj.def && obj.def.size == 1) {
      for (let [path, lines] of obj.def) {
        if (lines.length == 1) {
          jumps.set(id, {path, lineno: lines[0].lno, pretty: pretty.get(id)});
        }
      }
    }
  }

  os.file.close(redirect(old));

  old = redirect(jumpFile);

  for (let [id, {path, lineno, pretty}] of jumps) {
    print(JSON.stringify([id, path, lineno, pretty]));
  }

  os.file.close(redirect(old));
}

let filenamesString = snarf(filenamesFile);
let filenames = filenamesString.split("\n");

let filename;
for (filename of filenames) {
  processFile(filename);
}

writeMap();
