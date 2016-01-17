// Command line options:
// crossref.js <tree-root> <analysis-root> <output-file> <jump-file> file1 file2...
// File paths are relative to the tree roots.

let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let objdir = scriptArgs[3];
let filenamesFile = scriptArgs[4];

let analysisRoot = indexRoot + "/analysis";
let outputFile = indexRoot + "/crossref";
let jumpFile = indexRoot + "/jumps";

run(mozSearchRoot + "/output.js");

let identifiers = new Map();

function parseAnalysis(line, path)
{
  let parts = line.split(" ");
  if (parts.length != 4 && parts.length != 5) {
    throw `Invalid analysis line in ${path}: ${line}`;
  }

  if (parts[2][0] == '"') {
    parts[2] = eval(parts[2]);
  }

  let [linenum, colnum] = parts[0].split(":");
  linenum = parseInt(linenum);
  colnum = parseInt(colnum);
  return {line: linenum, col: colnum, kind: parts[1], name: parts[2], id: parts[3], extra: parts[4]};
}

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
  let analysis = snarf(analysisRoot + path);

  path = path.slice(1);

  let codeLines = code.split("\n");
  let analysisLines = analysis.split("\n");
  analysisLines.pop();

  function put(id, datum) {
    if (!identifiers.has(id)) {
      identifiers.set(id, {});
    }
    let obj = identifiers.get(id);
    if (!(datum.kind in obj)) {
      obj[datum.kind] = new Map();
    }
    let files = obj[datum.kind];
    if (!files.has(path)) {
      files.set(path, []);
    }
    files.get(path).push({ lno: datum.line, line: cut(codeLines[datum.line - 1].trim(), 100) });
  }

  for (let analysisLine of analysisLines) {
    let datum = parseAnalysis(analysisLine, path);
    put(datum.id, datum);
    if (datum.extra) {
      put(datum.extra, datum);
    }
  }
}

function writeMap()
{
  let jumps = new Set();

  function build(obj) {
    function buildKind(kind) {
      if (!obj[kind]) {
        return [];
      }
      let result = [ {path, lines} for ([path, lines] of obj[kind]) ];
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
      "Assignments": buildKind("assign")
    };
  }

  redirect(outputFile);

  for (let [id, obj] of identifiers) {
    print(id);
    print(JSON.stringify(build(obj)));

    if (obj.def && obj.def.size == 1) {
      for (let [path, lines] of obj.def) {
        if (lines.length == 1) {
          jumps.add(id);
        }
      }
    }
  }

  redirect(jumpFile);

  for (let id of jumps) {
    print(id);
  }
}

let filenamesString = snarf(filenamesFile);
let filenames = filenamesString.split("\n");

let filename;
for (filename of filenames) {
  processFile(filename);
}

writeMap();
