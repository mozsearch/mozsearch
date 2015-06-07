let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let paths = scriptArgs.slice(3);

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
  let listing = runCmd(`ls -l -L --time-style=long-iso "${path}"`);
  let lines = listing.split('\n');

  let entries = [];
  for (let line of lines.slice(1)) {
    if (!line.length) {
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

function generateDirectory(path, opt)
{
  let entries = listDirectory(treeRoot + path);

  let entryContent = "";
  for (let entry of entries) {
    let icon = "";

    if (entry.kind == "directory") {
      icon = "folder";
    } else {
      icon = chooseIcon(entry.name);
    }

    let relative_url = fileURL(opt.tree, path == "/" ? "/" + entry.name : path + "/" + entry.name);

    entryContent += `
        <tr>
          <td><a href="${relative_url}" class="icon ${icon}">${entry.name}</a></td>
          <td><a href="${relative_url}">${entry.date}</a></td>
          <td><a href="${relative_url}">${entry.size}</a></td>
        </tr>
`;
  }

  let content = `
  ${generateBreadcrumbs(path, opt)}

  <table class="folder-content">
    <thead>
      <tr>
        <th scope="col">Name</th>
        <th scope="col">Modified</th>
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

  redirect(indexRoot + "/dir" + path + "/index.html");
  print(output);
}

for (let dir of paths) {
  generateDirectory(dir, {tree: "mozilla-central", includeDate: true});
}
