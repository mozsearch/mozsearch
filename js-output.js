function parseAnalysis(line)
{
  let parts = line.split(" ");
  if (parts.length != 4 && parts.length != 5) {
    throw `Invalid analysis line: ${line}`;
  }

  let [linenum, colnum] = parts[0].split(":");
  return {line: linenum, col: colnum, kind: parts[1], name: parts[2], id: parts[3], extra: parts[4]};
}

let javascript = snarf(scriptArgs[0]);
let analysis = snarf(scriptArgs[1]);
let jumps = snarf(scriptArgs[2]);

let analysisLines = analysis.split("\n");
analysisLines.pop();
analysisLines.push("0:0 eof BAD BAD");

let jumpLines = jumps.split("\n");
jumps = new Set();
for (let id of jumpLines) {
  jumps.add(id);
}

let datum = parseAnalysis(analysisLines[0]);
let analysisIndex = 1;

let lines = javascript.split("\n");
let lineNum = 1;
for (let line of lines) {
  for (let col = 0; col < line.length; col++) {
    if (lineNum == datum.line && col == datum.col) {
      let extra = "";
      if (datum.extra) {
        extra += `data-extra="${datum.extra}" `;
      }
      if (jumps.has(datum.id)) {
        extra += `data-jump="true" `;
      }
      if (datum.extra && jumps.has(datum.extra)) {
        extra += `data-extra-jump="true" `;
      }
      putstr(`<span data-id="${datum.id}" data-kind=${datum.kind} ${extra}>${datum.name}</span>`);

      col += datum.name.length - 1;
      datum = parseAnalysis(analysisLines[analysisIndex++]);
    } else {
      let ch = line[col];
      if (ch == '<') {
        putstr("&lt;");
      } else {
        putstr(ch);
      }
    }
  }

  print("");

  lineNum++;
}

