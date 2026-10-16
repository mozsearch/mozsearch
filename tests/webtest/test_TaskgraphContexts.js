"use strict";

// Taskcluster's taskgraph YAML (see languages/semantic_mappings/
// taskcluster_*.toml): a task's name is an identifier (its pretty name is
// "<kind>::<name>"), and the records inside a task have the task as their
// context, with where they are in it (`ctxtra`, ex: "› fetches ›
// toolchain").  The tests tree's package kind's linux64/opt task fetches the
// toolchain linux64-clang.

add_task(async function test_TaskNameSearch() {
  await TestUtils.loadPath("/tests/search?q=linux64-clang");
  const content = frame.contentDocument.querySelector("#content");
  await waitForCondition(() => content.textContent.includes("Definitions (toolchain::linux64-clang)"),
                         "A task's name finds its definition");
  ok(content.textContent.includes("Uses (toolchain::linux64-clang)"), "and its uses");
  const contexts = [...content.querySelectorAll(".result-context")].map(node => node.textContent);
  ok(contexts.some(text => text.includes("found in package::linux64/opt") && text.includes("› fetches › toolchain")),
     "A use says which task it's in, and where in it");
});

add_task(async function test_TaskQueryPlacement() {
  await TestUtils.loadQuery("tests", "linux64-clang");
  const doc = frame.contentDocument;
  const contexts = [...doc.querySelectorAll(".result-context")].map(node => node.textContent);
  ok(contexts.some(text => text.includes("package::linux64/opt") && text.includes("› fetches › toolchain")),
     "/query/'s results say where in the task a use is");
  const facet = doc.querySelector('.facet[data-facet="ctxtra"]');
  ok(facet, "and facet the results by it");
  ok(facet?.querySelector('.facet-value[data-value="fetches"]'), "by its first key");
});
