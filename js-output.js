let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let filenames = scriptArgs.slice(3);

let window = this;
run(mozSearchRoot + "/highlight.pack.js");

function parseAnalysis(line)
{
  let parts = line.split(" ");
  if (parts.length != 4 && parts.length != 5) {
    throw `Invalid analysis line: ${line}`;
  }

  let [linenum, colnum] = parts[0].split(":");
  return {line: parseInt(linenum), col: parseInt(colnum),
          kind: parts[1], name: parts[2], id: parts[3], extra: parts[4]};
}

let jumps = snarf(indexRoot + "/jumps");
let jumpLines = jumps.split("\n");
jumps = new Set();
for (let id of jumpLines) {
  jumps.add(id);
}

function processFile(filename) {
  let t0 = Date.now();

  let javascript = snarf(treeRoot + "/" + filename);
  let analysis = snarf(indexRoot + "/analysis/" + filename);

  let analysisLines = analysis.split("\n");
  analysisLines.pop();
  analysisLines.push("100000000000:0 eof BAD BAD");

  let lastLine = -1;
  let lastCol = -1;

  let datum = parseAnalysis(analysisLines[0]);
  let analysisIndex = 1;

  javascript = hljs.highlight("js", javascript, true).value;

  let lines = javascript.split("\n");

  let t1 = Date.now();

  redirect(indexRoot + "/file/" + filename);

  let output = '';

  function out(s) {
    output += s;
    if (output.length > 4096) {
      putstr(output);
      output = '';
    }
  }

  out(`
<table id="file" class="file">
  <thead class="visually-hidden">
    <th scope="col">Line</th>
    <th scope="col">Code</th>
  </thead>
  <tbody>
    <tr>
      <td id="line-numbers">
`);

  let lineNum = 1;
  for (let line of lines) {
    out(`<span id="${lineNum}" class="line-number" unselectable="on" rel="#${lineNum}">${lineNum}</span>\n`);
    lineNum++;
  }

  out(`      </td>
      <td class="code">
<pre>`);

  let t2 = Date.now();

  function outputLine(lineNum, line) {
    let col = 0;
    for (let i = 0; i < line.length; i++) {
      let ch = line[i];

      if (ch == '&') {
        do {
          out(line[i]);
          i++;
        } while (line[i] != ';');
        out(line[i]);
        col++;
        continue;
      }

      if (ch == '<') {
        do {
          out(line[i]);
          i++;
        } while (line[i] != '>');
        out(line[i]);
        continue;
      }

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
        out(`<span data-id="${datum.id}" data-kind=${datum.kind} ${extra}>${datum.name}</span>`);

        col += datum.name.length - 1;
        i += datum.name.length - 1;
        do {
          datum = parseAnalysis(analysisLines[analysisIndex++]);
        } while (datum.line == lastLine && datum.col == lastCol);
        if (datum.line < lastLine || (datum.line == lastLine && datum.col < lastCol)) {
          throw `Invalid analysis loc: ${filename} ${JSON.stringify(datum)}`;
        }
        lastLine = datum.line;
        lastCol = datum.col;
      } else {
        out(ch);
      }

      col++;
    }
  }

  lineNum = 1;
  for (let line of lines) {
    out(`<code id="line-${lineNum}" aria-labelledby="${lineNum}">`);

    if (lineNum != datum.line) {
      out(line);
    } else {
      outputLine(lineNum, line);
    }

    out(`</code>\n`);

    lineNum++;
  }

  out(`</pre>
      </td>
    </tr>
  </tbody>
</table>
`);

  putstr(output);

  let t3 = Date.now();
  if (t3-t0 > 150)
  printErr(`${filename} ${t1-t0} ${t2-t1} ${t3-t2}`);
}

for (let filename of filenames) {
  processFile(filename);
}
