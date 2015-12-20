let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let filenames = scriptArgs.slice(3);

run(mozSearchRoot + "/output.js");

function runCmd(cmd)
{
  let outfile = `/tmp/cmd-output-${Math.random()}-${os.getpid()}`;
  os.system(`(${cmd}) > ${outfile}`);
  let data = snarf(outfile);
  os.system(`rm ${outfile}`);
  return data;
}

function parseAnalysis(line)
{
  let parts = line.split(" ");
  if (parts.length != 4 && parts.length != 5) {
    throw `Invalid analysis line: ${line}`;
  }

  if (parts[2][0] == '"') {
    parts[2] = eval(parts[2]);
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

function chooseLanguage(filename)
{
  let suffix = getSuffix(filename);

  let exclude = {'ogg': true, 'ttf': true, 'xpi': true, 'png': true, 'bcmap': true,
                 'gif': true, 'ogv': true, 'jpg': true, 'bmp': true, 'icns': true, 'ico': true,
                 'mp4': true, 'sqlite': true, 'jar': true, 'webm': true, 'woff': true,
                 'class': true, 'm4s': true, 'mgif': true, 'wav': true, 'opus': true,
                 'mp3': true, 'otf': true};
  if (suffix in exclude) {
    return "skip";
  }

  let table = {'js': 'javascript', 'jsm': 'javascript',
               'cpp': 'cpp', 'h': 'cpp', 'cc': 'cpp', 'hh': 'cpp', 'c': 'cpp',
               'py': 'python', 'sh': 'bash', 'build': 'python', 'ini': 'ini',
               'java': 'java', 'json': 'javascript', 'xml': 'xml', 'css': 'css',
               'html': 'html'};
  if (suffix in table) {
    return table[suffix];
  }
  return null;
}

function toHTML(code)
{
  code = code.replace("&", "&amp;", "gm");
  code = code.replace("<", "&lt;", "gm");
  return code;
}

function generatePanel(path)
{
  return `
  <div class="panel">
    <button id="panel-toggle">
      <span class="navpanel-icon expanded" aria-hidden="false"></span>
      Navigation
    </button>
    <section id="panel-content" aria-expanded="true" aria-hidden="false">

      <h4>Git</h4>
      <ul>
        <li>
          <a href="https://github.com/mozilla/gecko-dev/commits/master${path}" title="Log" class="log icon">Log</a>
        </li>
        <li>
          <a href="https://github.com/mozilla/gecko-dev/blame/master${path}" title="Blame" class="blame icon">Blame</a>
        </li>
        <li>
          <a href="https://raw.githubusercontent.com/mozilla/gecko-dev/master${path}" title="Raw" class="raw icon">Raw</a>
        </li>
      </ul>

      <h4>Mercurial</h4>
      <ul>
        <li>
          <a href="https://hg.mozilla.org/mozilla-central/filelog/tip${path}" title="Log" class="log icon">Log</a>
        </li>
        <li>
          <a href="https://hg.mozilla.org/mozilla-central/annotate/tip${path}" title="Blame" class="blame icon">Blame</a>
        </li>
        <li>
          <a href="https://hg.mozilla.org/mozilla-central/raw-file/tip${path}" title="Raw" class="raw icon">Raw</a>
        </li>
      </ul>

    </section>
  </div>
`;
}

function generateFile(path, opt)
{
  let language = chooseLanguage(path);

  let analysisLines = [];

  try {
    let analysis = snarf(indexRoot + "/analysis" + path);
    analysisLines = analysis.split("\n");
    analysisLines.pop();
  } catch (e) {
  }
  analysisLines.push("100000000000:0 eof BAD BAD");

  let lastLine = -1;
  let lastCol = -1;

  let datum = parseAnalysis(analysisLines[0]);
  let analysisIndex = 1;

  let code;
  if (language == "skip") {
    code = "binary file";
  } else if (language) {
    lineLen = parseInt(runCmd(`wc -L ${treeRoot + path}`));
    if (lineLen > 250) {
      try {
        code = snarf(treeRoot + path);
      } catch (e) {
        code = "binary file";
      }
      code = toHTML(code);
    } else {
      try {
        code = runCmd(`source-highlight --style-css-file=sh_ide-codewarrior.css -s ${language} -i ${treeRoot + path} | tail -n +5`);
      } catch (e) {
        code = "binary file";
      }
    }
  } else {
    try {
      code = snarf(treeRoot + path);
    } catch (e) {
      code = "binary file";
    }
    code = toHTML(code);
  }

  let lines = code.split("\n");

  let content = '';

  function out(s) {
    content += s;
  }

  out(generateBreadcrumbs(path, opt));
  out(generatePanel(path));

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
    out(`<span id="${lineNum}" class="line-number" unselectable="on">${lineNum}</span>\n`);
    lineNum++;
  }

  out(`      </td>
      <td class="code">
<pre>`);

  function esc(s) {
    return s.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;');
  }

  function outputLine(lineNum, line) {
    let col = 0;
    for (let i = 0; i < line.length; ) {
      let ch = line[i];

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
        out(`<span data-id="${datum.id}" data-kind=${datum.kind} ${extra}>${esc(datum.name)}</span>`);

        // Output the formatted link text.
        var stop = col + datum.name.length;
        while (col < stop) {
          ch = line[i];
          if (ch == '&') {
            do {
              i++;
            } while (line[i] != ';');
            col++;
            i++;
            continue;
          }

          if (ch == '<') {
            do {
              i++;
            } while (line[i] != '>');
            i++;
            continue;
          }

          col++;
          i++;
        }

        do {
          datum = parseAnalysis(analysisLines[analysisIndex++]);
        } while (datum.line == lastLine && datum.col == lastCol);
        if (datum.line < lastLine || (datum.line == lastLine && datum.col < lastCol)) {
          throw `Invalid analysis loc: ${path} ${JSON.stringify(datum)}`;
        }
        lastLine = datum.line;
        lastCol = datum.col;
      } else if (ch == '&') {
        do {
          out(line[i]);
          i++;
        } while (line[i] != ';');
        out(line[i]);
        col++;
        i++;
      } else if (ch == '<') {
        do {
          out(line[i]);
          i++;
        } while (line[i] != '>');
        out(line[i]);
        i++;
      } else {
        out(ch);
        col++;
        i++;
      }
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

  let fname = path.substring(path.lastIndexOf("/") + 1);
  opt.title = `${fname} - mozsearch`;

  redirect(indexRoot + "/file" + path);
  putstr(generate(content, opt));
}

for (let filename of filenames) {
  generateFile(filename, {tree: "mozilla-central", includeDate: true});
}
