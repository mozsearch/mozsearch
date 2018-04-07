let treeRoot = scriptArgs[0];
let indexRoot = scriptArgs[1];
let mozSearchRoot = scriptArgs[2];
let treeName = scriptArgs[3];

run(mozSearchRoot + "/scripts/output-lib.js");
run(mozSearchRoot + "/scripts/output.js");

let opt = {tree: treeName,
           title: "{{TITLE}} - mozsearch"};

let searchBody = `<script>
      var results = {{BODY}};
      window.addEventListener("load", function() { showSearchResults(results); });
    </script>`;

let output = generate(searchBody, opt);

let old = redirect(indexRoot + "/templates/search.html");
print(output);
os.file.close(redirect(old));
