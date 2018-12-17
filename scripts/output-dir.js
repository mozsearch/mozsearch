let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let hgRoot = scriptArgs[2];
let mozSearchRoot = scriptArgs[3];
let objdir = scriptArgs[4];
let treeName = scriptArgs[5];
let pathFiles = scriptArgs.slice(6);

run(mozSearchRoot + "/scripts/output-lib.js");
run(mozSearchRoot + "/scripts/output.js");

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
  try {
    this.description = snarf(descriptionPath(path));
  } catch (e) {
    // No description file
    this.description = "";
  }
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

function escapeHtml(unsafe) {
    return unsafe.replace(/&/g, "&amp;")
                 .replace(/</g, "&lt;")
                 .replace(/>/g, "&gt;")
                 .replace(/"/g, "&quot;")
                 .replace(/'/g, "&#039;");
}

function generateDirectory(dir, path, opt)
{
  let entries = Array.from(dir);
  entries.sort(compareFunc);

  let entryContent = "";
  for (let [filename, entry] of entries) {
    let icon = "";

    let size = "";
    let description = "";
    if (entry instanceof FileInfo) {
      icon = chooseIcon(filename);
      size = entry.size;
      description = entry.description;
    } else {
      icon = "folder";
      size = 0;
      for (let [subfile, subnode] of entry) {
        size++;
        // For folders, use the description from any
        // READMEs inside the folder
        switch (subfile) {
            case "README":
            case "README.txt":
            case "README.md":
                description = subnode.description;
                break;
        }
      }
    }

    description = escapeHtml(description);
    let filepath = path == "/" ? filename : path + "/" + filename;
    let relative_url = fileURL(opt.tree, filepath);
    let style = "";

    if (isIconForImage(icon)) {
      let hgPath = `${hgRoot}/raw-file/tip/${filepath}`;
      style = `background-image: url('${hgPath}');`;
    }

    entryContent += `
        <tr>
          <td><a href="${relative_url}" class="icon ${icon}" style="${style}">${filename}</a></td>
          <td class="description"><a href="${relative_url}" title="${description}">${description}</td>
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
        <th scope="col">Description</th>
        <th scope="col">Size</th>
      </tr>
    </thead>
    <tbody>
      ${entryContent}
    </tbody>
  </table>
`;

  let dirname = path == "/" ? "/" : path.substring(path.lastIndexOf("/") + 1);
  opt.title = `${dirname} - mozsearch`;

  let output = generate(content, opt);

  let old = redirect(indexRoot + "/dir/" + path + "/index.html");
  print(output);
  os.file.close(redirect(old));
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

    computeSizes(node, path == "" ? filename : path + "/" + filename);
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
  generateDirectory(dir, path == "" ? "/" : path, {tree: treeName, includeDate: true});

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
