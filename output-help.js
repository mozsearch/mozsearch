let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];

run(mozSearchRoot + "/output.js");

let opt = {tree: "mozilla-central",
           title: "Searchfox"};

let body = snarf(mozSearchRoot + "/help.html");
let output = generate(body, opt);

redirect(indexRoot + "/help.html");
print(output);
