"use strict";

// A file without the semantic analysis its language's files have in its
// tree has the "No Semantic Analysis" info box (see the tools'
// `semantic_coverage` and config_defaults/info_boxes/semantic.liquid): why,
// if it's a known gap, with its bug, else a warning, with a Bugzilla search.
// The tests tree's tests/semantic-coverage.toml has its Python, which
// scip-python analyzes but for python/coverage/'s files (see tests/tests/
// build): known_gap.py a known gap's, unexpected.py not.

function doc() {
  return frame.contentDocument;
}

add_task(async function test_SemanticCoverage() {
  await TestUtils.loadPath("/tests/source/python/coverage/known_gap.py");
  let box = doc().querySelector(".info-box-semantic");
  ok(box?.matches(".info-box-info.info-box-concise"), "A known gap's file has the info box");
  is(box.querySelector("h4")?.textContent, "No Semantic Analysis", "saying it has no semantic analysis");
  ok(box.textContent.includes("scip-python doesn't analyze this Python file"), "by its language's analyzer");
  ok(box.textContent.includes("to test this info box"), "and why");
  is(box.querySelector("a.info-box-details")?.getAttribute("href"),
     "https://bugzilla.mozilla.org/show_bug.cgi?id=1999118", "with its bug");

  await TestUtils.loadPath("/tests/source/python/coverage/unexpected.py");
  box = doc().querySelector(".info-box-semantic");
  ok(box?.matches(".info-box-warning"), "A file which isn't a known gap's has a warning");
  ok(box.textContent.includes("This tree's Python files are analyzed with scip-python, but this one has no analysis"),
     "saying it's unexpected");
  is(box.querySelector("a.info-box-details")?.getAttribute("href"),
     "https://bugzilla.mozilla.org/buglist.cgi?quicksearch=searchfox%20python", "with a Bugzilla search");

  await TestUtils.loadPath("/tests/source/some_python.py");
  ok(doc().querySelector('[data-symbols^="S_py_"]'), "A Python file with its analysis");
  ok(!doc().querySelector(".info-box-semantic"), "has no such box");
  await TestUtils.loadPath("/tests/source/taskcluster/gecko_taskgraph/target_tasks.py");
  ok(!doc().querySelector(".info-box-semantic"), "(nor one with a semantic mapping's records too)");
});
