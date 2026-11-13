"use strict";

// The page reloads on a popstate event (see search.js's popstate handler) only
// if its content isn't for the URL it goes to: when it goes from or to a
// dynamic search's results, or to a URL other scripts changed with the
// history API (ex: an interactive diagram's query, after ignoring one of its
// nodes); but not for navigations to a fragment of the page.

// Go back, and whether the page loads again (in 5 s).
async function backReloads() {
  const loaded = TestUtils.waitForLoad().then(() => true);
  frame.contentWindow.history.back();
  return Promise.race([loaded, new Promise(resolve => setTimeout(() => resolve(false), 5000))]);
}

// Navigations to a fragment of the page fire popstate events (as does Enter
// in the URL bar for the page's URL with a fragment, bug 2092563's case, which
// a test can't do), which scroll, without reloading the page, as going back
// does.
add_task(async function test_PopStateFragment() {
  await TestUtils.loadPath("/tests/source/big_cpp.cpp#136");
  const win = frame.contentWindow;
  win.notReloaded = true;
  let popstates = 0;
  win.addEventListener("popstate", () => popstates++);
  const link = frame.contentDocument.createElement("a");
  link.href = "#200";
  link.append("Line 200");
  frame.contentDocument.querySelector("#panel").append(link);
  link.click();
  await waitForCondition(() => popstates > 0 && win.location.hash == "#200",
                         "Going to another line fires a popstate event");
  await new Promise(resolve => setTimeout(resolve, 500));
  ok(frame.contentWindow.notReloaded, "which doesn't reload the page");

  win.history.back();
  await waitForCondition(() => frame.contentWindow.location.hash == "#136", "Going back");
  await new Promise(resolve => setTimeout(resolve, 500));
  ok(frame.contentWindow.notReloaded, "doesn't either");
});

// A dynamic search's results are the page's content, for the URL the search
// pushes, so going back to the page's own URL reloads it.
add_task(async function test_PopStateDynamicSearch() {
  await TestUtils.loadPath("/tests/source/big_cpp.cpp");
  TestUtils.shortenSearchTimeouts();
  frame.contentWindow.notReloaded = true;
  TestUtils.setText(frame.contentDocument.querySelector("#query"), "SimpleSearch");
  await waitForCondition(() => frame.contentDocument.location.href.includes("SimpleSearch"),
                         "The search pushes its URL");

  ok(await backReloads() && !frame.contentWindow.notReloaded, "Going back reloads the page");
  ok(frame.contentDocument.location.pathname.endsWith("/big_cpp.cpp") &&
     frame.contentDocument.querySelector("#line-1"),
     "which shows the file again");
});

// An interactive diagram leaves out a node it's told to ignore and pushes the
// query ignoring it, so going back reloads the page, whose diagram has it
// again.
add_task(async function test_PopStateDiagramIgnore() {
  await TestUtils.resetFeatureGate("diagramming");
  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");
  const F5 = '[data-symbols="_ZN14diagram_ignore2F5Ev"]';
  const doc = frame.contentDocument;
  frame.contentWindow.notReloaded = true;
  TestUtils.click(doc.querySelector(F5));
  const ignoreItem = [...doc.querySelectorAll(".contextmenu-row a")]
    .find(a => a.textContent.includes("Ignore this node"));
  // (Cancelable, as a real click is, whose default, going to the item's "#",
  // the menu prevents.)
  TestUtils.click(ignoreItem, { bubbles: true, cancelable: true });
  await waitForCondition(() => !doc.querySelector(F5), "Ignoring a node leaves it out");
  ok(frame.contentWindow.location.search.includes("ignore-nodes"), "and pushes the query ignoring it");
  ok(frame.contentWindow.notReloaded, "without reloading the page");

  ok(await backReloads() && !frame.contentWindow.notReloaded, "Going back reloads the page");
  ok(!frame.contentWindow.location.search.includes("ignore-nodes"), "for the query without it");
  await waitForCondition(() => frame.contentDocument.querySelector(F5), "whose diagram has the node again");
});
