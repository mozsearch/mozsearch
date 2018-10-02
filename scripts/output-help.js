let helpFile = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let treeName = scriptArgs[3];

run(mozSearchRoot + "/scripts/output-lib.js");
run(mozSearchRoot + "/scripts/output.js");

let opt = {tree: treeName,
           title: "Searchfox",
           autofocusSearch: true};

let body = snarf(helpFile);
let output = generate(body, opt);

let old = redirect(indexRoot + "/help.html");
print(output);
os.file.close(redirect(old));
