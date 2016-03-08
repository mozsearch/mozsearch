function getSuffix(filename)
{
  let pos = filename.lastIndexOf(".");
  if (pos == -1) {
    return null;
  }
  return filename.slice(pos + 1).toLowerCase();
}

function chooseIcon(path)
{
  let suffix = getSuffix(path);
  return { 'cpp': 'cpp', 'h': 'h', 'c': 'c', 'js': 'js', 'jsm': 'js', 'py': 'py' }[suffix] || "";
}

function readAnalysis(filePath, keep)
{
  let text = snarf(filePath);
  let lines = text.split("\n");
  lines.pop();

  let result = {sources: [], targets: []};

  let dummyCol = [0, 0];
  let analysisLine = 0;
  for (let l of lines) {
    analysisLine++;
    let j;
    try {
      j = JSON.parse(l);
    } catch (e) {
      print(`Bad JSON: ${filePath}:${analysisLine}. ${e}`);
    }
    if (!keep(j)) {
      continue;
    }

    let [line, col] = j.loc.split(":");
    if (col.indexOf("-") != -1) {
      [col1, col2] = col.split("-");
      col1 = parseInt(col1);
      col2 = parseInt(col2);
      col = [col1, col2];
    } else {
      col = parseInt(col);
      col = [col, col];
    }
    line = parseInt(line);
    j.loc = {line, col};

    if (j.source) {
      result.sources.push(j);
    } else if (j.target) {
      result.targets.push(j);
    }
  }

  function sortAnalysis(list) {
    list.sort(function(r1, r2) {
      if (r1.loc.line == r2.loc.line) {
        return r1.loc.col[0] - r2.loc.col[0];
      } else {
        return r1.loc.line - r2.loc.line;
      }
    });

    let result = [];
    let pushed = {};
    let lastLoc = null;
    let lastElt = null;
    for (let j of list) {
      let loc = j.loc;
      if (lastLoc && loc.line == lastLoc.line && loc.col[0] == lastLoc.col[0]) {
        let s = JSON.stringify(j);
        if (!(s in pushed)) {
          pushed[s] = true;
          lastElt.push(j);
        }
      } else {
        lastLoc = loc;

        pushed = {};
        let s = JSON.stringify(j);
        pushed[s] = true;

        let r = {loc, analysis: [j]};
        lastElt = r.analysis;
        result.push(r);
      }
    }

    return result;
  }

  result.sources = sortAnalysis(result.sources);
  result.targets = sortAnalysis(result.targets);

  return result;
}
