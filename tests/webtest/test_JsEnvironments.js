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
  is(worker.querySelector("li[aria-current]")?.textContent.replace(/\s+/g, " ").trim(),
     "jsenv/worker_helper.js loaded at worker_jsenv.js:1", "saying \"loaded\" once");
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

// A test's function its harness calls by its name, in their global, is a
// hook's definition (see scip-typescript's --hooks): the head's calls of
// run_test are the hook's uses, whose context menu searches for its
// definitions (test_shared.js's and test_other.js's), rather than jumping to
// one of them.
add_task(async function test_JsEnvironmentsHooks() {
  await TestUtils.loadPath("/tests/source/jsenv/unit/head_jsenv.js");
  const hook = "S_js_typescript_test-ts/jsenv/unit/head_jsenv.js/run_test:";
  const call = [...frame.contentDocument.querySelectorAll("#line-10 span[data-symbols]")]
    .find(span => span.textContent == "run_test");
  is(call?.dataset.symbols, hook, "The head's call of its test's run_test is a hook's use");
  TestUtils.click(call);
  const menu = frame.contentDocument.querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown for the hook");
  const links = [...menu.querySelectorAll("a")];
  ok(links.some(a => a.textContent == "Search for hook jsenv/unit/head_jsenv.js::run_test"),
     "which offers a search for the hook's definitions");
  ok(!links.some(a => a.textContent.startsWith("Go to definition")),
     "rather than jumping to one of them");

  await TestUtils.loadPath("/tests/source/jsenv/unit/test_shared.js");
  ok(symbolsOf("run_test").split(",").includes(hook), "The test's run_test is a definition of the hook");
});

// A test's page's calls diagrams are of its JS environments' code (`test:`;
// see tools/src/js_environments/scope.rs), by symbol, as its run_test's
// pretty name is its other tests' too; and so are those of a page with a
// `?test=`.
add_task(async function test_JsEnvironmentsDiagramContext() {
  const diagramQuery = async (path, line, name, label) => {
    await TestUtils.loadPath(path);
    const span = [...frame.contentDocument.querySelectorAll(`#line-${line} span[data-symbols]`)]
      .find(span => span.textContent == name);
    TestUtils.click(span);
    const menu = frame.contentDocument.querySelector("#context-menu");
    await waitForShown(menu, `Context menu is shown for ${name}`);
    const link = [...menu.querySelectorAll("a")].find(a => a.textContent == label);
    return link && new URL(link.href).searchParams.get("q");
  };
  is(await diagramQuery("/tests/source/jsenv/unit/test_shared.js", 4, "run_test", "Calls to"),
     "calls-to-sym:'S_js_typescript_test-ts/jsenv/unit/test_shared.js/run_test().' depth:4 " +
     "test:'jsenv/unit/test_shared.js'",
     "A test's calls diagrams are of its environments' code, by symbol");
  is(await diagramQuery("/tests/source/jsenv/unit/head_jsenv.js?test=jsenv/unit/test_shared.js", 8,
                        "runTests", "Calls from"),
     "calls-from-sym:'S_js_typescript_test-ts/jsenv/unit/head_jsenv.js/runTests().' depth:4 " +
     "test:'jsenv/unit/test_shared.js'",
     "and so are a page's with a test context");
  ok(!(await diagramQuery("/tests/source/jsenv/unit/head_jsenv.js", 8, "runTests", "Calls from"))
       .includes("test:"),
     "but not a harness's without one");
});

// A web-platform-tests page's inline scripts and event handlers are analyzed
// in its environment, as a file of their own whose analysis is the page's
// (see scan.rs's `inline_file`, and scip-indexer's --inline-pages): its
// handler's call is of its inline script's function, a symbol of the page's,
// and its test() is testharness.js's; and its styles' custom properties are
// its own (see css_analyzer.rs's `document_namespace`), as a stylesheet's
// are the stylesheet's.
function symbolsOn(line, name) {
  return [...frame.contentDocument.querySelectorAll(`#line-${line} span[data-symbols]`)]
    .find(span => span.textContent == name)?.dataset.symbols || "";
}

add_task(async function test_JsEnvironmentsInlineScripts() {
  await TestUtils.loadPath("/tests/source/testing/web-platform/tests/jsenv-pages/inline-scripts.html");
  ok(section()?.querySelector(".js-env-roots li[data-root='testing/web-platform/tests/jsenv-pages/inline-scripts.html']"),
     "A web-platform-tests page has its environment");
  const clicked = "testing/web-platform/tests/jsenv-pages/inline-scripts.html/clicked().";
  ok(symbolsOn(9, "clicked").endsWith(clicked), "whose event handler's call is of the page's function");
  ok(symbolsOn(11, "clicked").endsWith(clicked), "its inline script's definition");
  ok(symbolsOn(15, "test").endsWith("testing/web-platform/tests/resources/testharness.js/test()."),
     "and its inline script's test() is its harness's");
  ok(!frame.contentDocument.querySelector("#file span[data-symbols^='#']"),
     "rather than of js-analyze's names");
  ok([...frame.contentDocument.querySelectorAll("#line-6 span[data-symbols]")].some(span =>
       span.dataset.symbols == "CSSPROP_testing/web@2Dplatform/tests/jsenv@2Dpages/inline@2Dscripts@2Ehtml/@2D@2Dinline@2Dgap"),
     "Its styles' custom properties are its own");

  await TestUtils.loadPath("/tests/source/testing/web-platform/tests/jsenv-pages/support/linked.css");
  ok([...frame.contentDocument.querySelectorAll("#line-3 span[data-symbols]")].some(span =>
       span.dataset.symbols == "CSSPROP_testing/web@2Dplatform/tests/jsenv@2Dpages/support/linked@2Ecss/@2D@2Dlinked@2Dgap"),
     "and a test's stylesheet's are its own");
});

// A `.worker.js` test loads its harness itself, and a Test262 test runs
// after its harness and its frontmatter's includes.
add_task(async function test_JsEnvironmentsWptKinds() {
  await TestUtils.loadPath("/tests/source/testing/web-platform/tests/jsenv-pages/worker-test.worker.js");
  is(scripts(section().querySelector(".js-env-roots > li.js-env")).join(", "),
     "testing/web-platform/tests/jsenv-pages/worker-test.worker.js test, " +
     "testing/web-platform/tests/resources/testharness.js loaded",
     "A .worker.js test's environment has the harness it loads");
  ok(symbolsOn(5, "test").endsWith("testing/web-platform/tests/resources/testharness.js/test()."),
     "whose test() it calls");

  await TestUtils.loadPath("/tests/source/testing/web-platform/tests/third_party/test262/test/built-ins/Array/compare.js");
  ok(symbolsOn(7, "compareArray").endsWith("testing/web-platform/tests/third_party/test262/harness/compareArray.js/compareArray()."),
     "A Test262 test's call is of its include's function");
  ok(symbolsOn(7, "assert").endsWith("testing/web-platform/tests/third_party/test262/harness/assert.js/assert()."),
     "and of its harness's");
});


// js-analyze's HTML pass reads all of a page's elements though it omits its
// optional <html> (as most web-platform-tests do), whose first top-level
// element's end sax took as the document's (see sax.js's multipleRoots): ex,
// this page's <style>, after its <title>.
add_task(async function test_HtmlAnalysisAfterFirstElement() {
  await TestUtils.loadPath("/tests/source/testing/web-platform/tests/jsenv-pages/inline-scripts.html");
  ok([...frame.contentDocument.querySelectorAll("#line-6 span[data-symbols]")]
       .some(span => span.dataset.symbols.startsWith("CSSPROP_")),
     "A page's style after its first element has its analysis");
});
