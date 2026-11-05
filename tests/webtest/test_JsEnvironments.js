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
