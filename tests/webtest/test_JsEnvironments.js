"use strict";

// A JS file's environments, in its navigation panel (see
// tools/src/js_environments/ and panel_sections/tests_environments.liquid):
// tests/files/jsenv/unit/test_shared.js runs under xpcshell (after its
// manifest's head, head_jsenv.js) and in test_shared.html's page (with
// helpers.js), which starts a worker, worker_jsenv.js, importing
// worker_helper.js.

function section() {
  return frame.contentDocument.querySelector(".panel-js-environments");
}

function symbolsOf(name) {
  return [...frame.contentDocument.querySelectorAll("#file span[data-symbols]")]
    .find(span => span.textContent == name)?.dataset.symbols || "";
}

function scripts(env) {
  return [...env.querySelectorAll(":scope > ol.js-env-scripts > li")]
    .map(li => `${li.querySelector("a").textContent} ${li.querySelector(".js-env-role").textContent}`);
}

add_task(async function test_JsEnvironmentsOfTest() {
  await TestUtils.loadPath("/tests/source/jsenv/unit/test_shared.js");
  ok(section(), "A test's panel has its JS environments");
  const root = section().querySelector(".js-env-roots > li.js-env");
  is(root?.querySelector(".js-env-flavor").textContent, "xpcshell", "Its own, an xpcshell test's");
  is(root.dataset.root, "jsenv/unit/test_shared.js", "of which it's the root");
  is(scripts(root).join(", "), "jsenv/unit/head_jsenv.js head, jsenv/unit/test_shared.js test",
     "after its manifest's head file");
  ok(root.querySelector("li[aria-current] a")?.textContent == "jsenv/unit/test_shared.js",
     "itself marked");
  is(root.querySelector(".js-env-manifest a")?.getAttribute("href"), "/tests/source/jsenv/unit/xpcshell.toml",
     "with its manifest");

  const page = section().querySelector(".js-env-ins > li.js-env-in");
  is(page?.dataset.root, "jsenv/test_shared.html", "It's also in a mochitest page's environment");
  is(page.querySelector(".js-env-role").textContent, "page", "as one of its scripts");
  const tree = page.querySelector("details.js-env-tree");
  ok(tree && !tree.open, "whose environments are folded");
  is(scripts(tree).join(", "),
     "jsenv/unit/test_shared.js page, jsenv/helpers.js page, jsenv/test_shared.html inline",
     "and list its other scripts, ex: the other helper");
  is(tree.querySelector("ol.js-env-scripts a")?.getAttribute("href"),
     "/tests/source/jsenv/unit/test_shared.js?test=jsenv/test_shared.html",
     "whose links keep the test as their context");
});

add_task(async function test_JsEnvironmentsOfWorkerScript() {
  await TestUtils.loadPath("/tests/source/jsenv/worker_helper.js?test=jsenv/test_shared.html");
  const entry = section()?.querySelector(".js-env-ins > li.js-env-in");
  ok(entry?.textContent.includes("loaded"), "A script a worker imports is in the worker's environment");
  ok(entry.classList.contains("js-env-current"), "which ?test= marks");
  const tree = entry.querySelector("details.js-env-tree");
  ok(tree?.open, "and opens");
  const worker = tree.querySelector(".js-env-children > li.js-env-child");
  ok(worker?.textContent.includes("worker started at test_shared.html:10"),
     "The page's tree has the worker it starts, where");
  is(scripts(worker).join(", "), "jsenv/worker_jsenv.js test, jsenv/worker_helper.js loaded",
     "with the scripts loaded into it");
  ok(worker.querySelector("li[aria-current] .js-env-from a")?.getAttribute("href")
       .endsWith("/tests/source/jsenv/worker_jsenv.js#1"),
     "where it's loaded from");
});

// Their scripts are analyzed with scip-typescript in their environments (see
// scripts/js-environments-analyze.sh), merged as platforms': the test's call
// of sharedHelper is of both of its environments' definitions.
add_task(async function test_JsEnvironmentsAnalysis() {
  await TestUtils.loadPath("/tests/source/jsenv/unit/test_shared.js");
  const call = [...frame.contentDocument.querySelectorAll("#line-5 span[data-symbols]")]
    .find(span => span.textContent == "sharedHelper");
  const symbols = call?.dataset.symbols.split(",") || [];
  ok(symbols.some(s => s.endsWith("jsenv/unit/head_jsenv.js/sharedHelper().")),
     "A test's call is of its xpcshell environment's definition");
  ok(symbols.some(s => s.endsWith("jsenv/helpers.js/sharedHelper().")),
     "and of its mochitest page's");
});

// A browser-chrome test runs in its harness's scope (testScope; see
// js-environments.toml's scope), whose members are the names it sees.
add_task(async function test_JsEnvironmentsScope() {
  await TestUtils.loadPath("/tests/source/jsenv/browser/browser_scope.js");
  ok(symbolsOf("ok").endsWith("jsenv/harness/browser-test.js/testScope().ok."),
     "The scope's members are its constructor's");
  ok(symbolsOf("add_task").endsWith("jsenv/harness/browser-test.js/testScope().prototype.add_task()."),
     "and its prototype's");
  ok(symbolsOf("browserHelper").endsWith("jsenv/browser/head.js/browserHelper()."),
     "and the test's head file's functions are globals");
});

// A JS shell test's helpers are its directories' shell.js's, which defines
// them in a function it calls at once (unwrapped; see js-environments.toml's
// `unwrap`), and those of a script it loads relative to itself.
add_task(async function test_JsEnvironmentsUnwrap() {
  await TestUtils.loadPath("/tests/source/jsenv/shell/tests/test_helpers.js");
  ok(symbolsOf("assertSame").endsWith("jsenv/shell/shell.js/assertSame()."),
     "A function its harness exposes is the harness's");
  ok(symbolsOf("assertAbout").endsWith("jsenv/shell/shell.js/assertAbout."),
     "and so is one it assigns to its global");
  ok(symbolsOf("assertNear").endsWith("jsenv/shell/lib/near.js/assertNear."),
     "and the loaded script's");
  ok(symbolsOf("Error") && !symbolsOf("Error").includes("jsenv/shell"),
     "but not the builtin the harness keeps a reference to");
});
