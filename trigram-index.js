let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let filenamesFile = scriptArgs[2];

let filenames = snarf(filenamesFile).split("\n");

let index = Object.create(null);

function getTrigram(trigram)
{
  if (!(trigram in index)) {
    index[trigram] = [];
  }
  return index[trigram];
}

for (let i = 0; i < filenames.length; i++) {
  let filename = filenames[i];

  print(`${i} / ${filenames.length}`);

  let data = snarf(treeRoot + "/" + filename).split("\n");
  for (let n = 0; n < data.length; n++) {
    let line = data[n];

    for (let col = 0; col <= line.length - 3; col++) {
      let trigram = line.substr(col, 3);
      let arr = getTrigram(trigram);
      arr.push(i);
      arr.push(n);
      arr.push(col);
    }
  }
}
