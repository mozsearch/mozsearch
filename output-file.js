"use strict";

let treeRoot = scriptArgs[0];
let treeRev = scriptArgs[1];
let indexRoot = scriptArgs[2];
let mozSearchRoot = scriptArgs[3];
let objdir = scriptArgs[4];
let filenames = scriptArgs.slice(5);

run(mozSearchRoot + "/lib.js");
run(mozSearchRoot + "/output.js");

function runCmd(cmd)
{
  let outfile = `/tmp/cmd-output-${Math.random()}-${os.getpid()}`;
  os.system(`(${cmd}) > ${outfile}`);
  let data = snarf(outfile);
  os.system(`rm ${outfile}`);
  return data;
}

let jumps = snarf(indexRoot + "/jumps");
let jumpLines = jumps.split("\n");
jumps = new Map();
for (let line of jumpLines) {
  if (!line.length) {
    continue;
  }

  let id, path, lineno, pretty;
  [id, path, lineno, pretty] = JSON.parse(line);
  jumps.set(id, [path, lineno, pretty]);
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
  code = code.replace(/&/gm, "&amp;");
  code = code.replace(/</gm, "&lt;");
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

      <h4>Mercurial</h4>
      <ul>
        <li>
          <a href="javascript:blame_link('/mozilla-central/commit/${treeRev}/${path}')" title="Blame/Permalink" class="blame icon">Blame/Permalink</a>
        </li>
      </ul>

    </section>
  </div>
`;
}

function generateFile(path, opt)
{
  let language = chooseLanguage(path);

  let analysis = [];
  try {
    let r = readAnalysis(indexRoot + "/analysis/" + path, j => j.source);
    analysis = r.sources;
  } catch (e) {
    printErr(e);
  }
  analysis.push({loc: {line: 100000000000, col: 0}});

  let datum = analysis[0];
  let analysisIndex = 1;

  let source = sourcePath(path);

  let code;
  if (language == "skip") {
    code = "binary file";
  } else if (language) {
    let lineLen = parseInt(runCmd(`wc -L ${source}`));
    if (lineLen > 250) {
      try {
        code = snarf(source);
      } catch (e) {
        code = "binary file";
      }
      code = toHTML(code);
    } else {
      try {
        code = runCmd(`source-highlight --style-css-file=sh_ide-codewarrior.css -s ${language} -i ${source} | tail -n +5`);
      } catch (e) {
        code = "binary file";
      }
    }
  } else {
    try {
      code = snarf(source);
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

  let generatedJSON = [];

  function outputLine(lineNum, line) {
    let col = 0;
    for (let i = 0; i < line.length; ) {
      let ch = line[i];

      if (lineNum == datum.loc.line && col == datum.loc.col[0]) {
        let id = datum.analysis[0].sym;

        let menuJumps = new Map();

        for (let r of datum.analysis) {
          let syms = r.sym.split(",");
          for (let sym of syms) {
            if (jumps.has(sym)) {
              let [jPath, jLineno, pretty] = jumps.get(sym);
              let key = jPath + ":" + jLineno;
              if (path != jPath || lineNum != jLineno) {
                menuJumps.set(key, {sym, pretty});
              }
            }
          }
        }

        let index = analysisIndex - 1;
        generatedJSON[index] = [Array.from(menuJumps.values()), datum.analysis];
        out(`<span data-i="${index}" data-id="${id}">`);

        // Output the formatted link text.
        let stop = datum.loc.col[1];
        let text = "";
        while (col < stop) {
          ch = line[i];
          if (ch == '&') {
            do {
              text += line[i];
              i++;
            } while (line[i] != ';');
            text += line[i];
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

          text += ch;
          col++;
          i++;
        }

        out(text);
        out("</span>");

        datum = analysis[analysisIndex++];
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

    if (lineNum != datum.loc.line) {
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

  out(`<script>
var ANALYSIS_DATA = ${JSON.stringify(generatedJSON)};
</script>
`);

  let fname = path.substring(path.lastIndexOf("/") + 1);
  opt.title = `${fname} - mozsearch`;

  let old = redirect(indexRoot + "/file/" + path);
  putstr(generate(content, opt));
  os.file.close(redirect(old));
}

for (let filename of filenames) {
  generateFile(filename, {tree: "mozilla-central", includeDate: true});
}
