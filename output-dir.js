let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let objdir = scriptArgs[3];
let pathFiles = scriptArgs.slice(4);

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

function listDirectory(path)
{
  let listing = runCmd(`ls -la -L --time-style=long-iso "${path}"`);
  let lines = listing.split('\n');

  let entries = [];
  for (let line of lines) {
    if (!line.length || line.startsWith("total ")) {
      continue;
    }

    let kind = line[0] == "d" ? "directory" : "file";
    let pieces = line.split(/\s+/, 8);
    let size = parseInt(pieces[4]);
    let date = `${pieces[5]} ${pieces[6]}`;
    let name = pieces[7];
    entries.push({kind, size, date, name});
  }
  return entries;
}

function FileInfo(path)
{
  this.path = path;
  this.size = 0;
}

// neg if 1 is before 2
function compareFunc([filename1, entry1], [filename2, entry2])
{
  function prio(filename) {
    if (filename == "__GENERATED__") {
      return 0;
    } else if (filename[0] == ".") {
      return 1;
    } else {
      return 2;
    }
  }

  let prio1 = prio(filename1);
  let prio2 = prio(filename2);

  let cmp = filename1.localeCompare(filename2);
  if (prio1 < prio2) {
    cmp += 1000;
  } else if (prio1 > prio2) {
    cmp -= 1000;
  }
  return cmp;
}

function generateDirectory(dir, path, opt)
{
  let entries = Array.from(dir);
  entries.sort(compareFunc);

  let entryContent = "";
  for (let [filename, entry] of entries) {
    let icon = "";

    let size = "";
    if (entry instanceof FileInfo) {
      icon = chooseIcon(filename);
      size = entry.size;
    } else {
      icon = "folder";
      size = 0;
      for (let x of entry) {
        size++;
      }
    }

    let relative_url = fileURL(opt.tree, path == "/" ? "/" + filename : path + "/" + filename);

    entryContent += `
        <tr>
          <td><a href="${relative_url}" class="icon ${icon}">${filename}</a></td>
          <td><a href="${relative_url}">${size}</a></td>
        </tr>
`;
  }

  let content = `
  ${generateBreadcrumbs(path, opt)}

  <table class="folder-content">
    <thead>
      <tr>
        <th scope="col">Name</th>
        <th scope="col">Size</th>
      </tr>
    </thead>
    <tbody>
      ${entryContent}
    </tbody>
  </table>
`;

  let dirname = path.substring(path.lastIndexOf("/") + 1);
  opt.title = `${dirname} - mozsearch`;

  let output = generate(content, opt);

  redirect(indexRoot + "/dir/" + path + "/index.html");
  print(output);
}

function addFile(filename, structure)
{
  let components = filename.split("/");
  let m = structure;
  for (let component of components.slice(0, -1)) {
    if (component == "") throw "BAD";
    if (m.has(component)) {
      m = m.get(component);
    } else {
      let m2 = new Map();
      m.set(component, m2);
      m = m2;
    }
  }

  let last = components[components.length - 1];
  m.set(last, new FileInfo(filename));
}

function computeSizes(dir, path)
{
  let entries = listDirectory(sourcePath(path));
  for (let entry of entries) {
    let e = dir.get(entry.name);
    if (!e || !(e instanceof FileInfo)) {
      continue;
    }

    e.size = entry.size;
  }

  for (let [filename, node] of dir) {
    if (node instanceof FileInfo) {
      continue;
    }

    computeSizes(node, path + "/" + filename);
  }
}

function readPathFile(pathFile, structure)
{
  let filenamesString = snarf(pathFile);
  let filenames = filenamesString.split("\n");

  let filename;
  for (filename of filenames) {
    if (filename == "") {
      continue;
    }
    addFile(filename, structure);
  }
}

function recursiveGenerate(dir, path)
{
  generateDirectory(dir, path == "" ? "/" : path, {tree: "mozilla-central", includeDate: true});

  for (let [filename, node] of dir) {
    if (node instanceof FileInfo) {
      continue;
    }

    recursiveGenerate(node, path == "" ? filename : path + "/" + filename);
  }
}

let structure = new Map();
for (let pathFile of pathFiles) {
  readPathFile(pathFile, structure);
}

computeSizes(structure, "");
recursiveGenerate(structure, "");
