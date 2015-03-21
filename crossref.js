// Command line options:
// crossref.js <source-tree-root> <analysis-root> <output-file> <jump-file> file1 file2...
// File paths are relative to the tree roots.

let sourceRoot = scriptArgs[0];
let analysisRoot = scriptArgs[1];
let outputFile = scriptArgs[2];
let jumpFile = scriptArgs[3];
let filenames = scriptArgs.slice(4);

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

function processFile(filename)
{
  let code = snarf(sourceRoot + "/" + filename);
  let analysis = snarf(analysisRoot + "/" + filename);

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
    if (!files.has(filename)) {
      files.set(filename, []);
    }
    files.get(filename).push({ n: datum.line, ex: cut(codeLines[datum.line - 1].trim(), 50) });
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
      let result = [ {filename, lines} for ([filename, lines] of obj[kind]) ];
      if (result.length > 1000) {
        return result.slice(0, 1000);
      } else {
        return result;
      }
    }

    return {
      use: buildKind("use"),
      def: buildKind("def"),
      assign: buildKind("assign")
    };
  }

  redirect(outputFile);

  for (let [id, obj] of identifiers) {
    print(id);
    print(JSON.stringify(build(obj)));

    if (obj.def && obj.def.size == 1) {
      for (let [filename, lines] of obj.def) {
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
