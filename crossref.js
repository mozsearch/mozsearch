// Command line options:
// crossref.js <source-tree-root> <analysis-root> <output-file> <jump-file> file1 file2...
// File paths are relative to the tree roots.

let sourceRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let filenames = scriptArgs.slice(3);

let analysisRoot = indexRoot + "/analysis";
let outputFile = indexRoot + "/crossref";
let jumpFile = indexRoot + "/jumps";

run(mozSearchRoot + "/output.js");

let identifiers = new Map();

function parseAnalysis(line)
{
  let parts = line.split(" ");
  if (parts.length != 4 && parts.length != 5) {
    throw `Invalid analysis line: ${line}`;
  }

  let [linenum, colnum] = parts[0].split(":");
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
  let code = snarf(sourceRoot + path);
  let analysis = snarf(analysisRoot + path);

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
    let datum = parseAnalysis(analysisLine);
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

for (let filename of filenames) {
  processFile(filename);
}

writeMap();
