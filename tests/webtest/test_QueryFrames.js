"use strict";

// Results as one types in a query box (see query-frames.js): in a tree
// searching with queries (trydelta), and on queries' pages, a pause in typing
// a query worth searching as it's typed loads its page in a frame over the
// page, which Enter makes the page, and Escape or Back takes away.

function doc() {
  return frame.contentDocument;
}

function win() {
  return frame.contentWindow;
}

// (Type `text` at the box's end, as the keyboard does; see
// test_QueryQuoting.js.)
function type(input, text) {
  const w = input.ownerDocument.defaultView;
  input.focus();
  input.setSelectionRange(input.value.length, input.value.length);
  for (const data of text) {
    const before = new w.InputEvent("beforeinput", {
      bubbles: true, cancelable: true, inputType: "insertText", data,
    });
    if (input.dispatchEvent(before)) {
      input.setRangeText(data, input.selectionStart, input.selectionEnd, "end");
      input.dispatchEvent(new w.InputEvent("input", {
        bubbles: true, inputType: "insertText", data,
      }));
    }
  }
}

function escape(input) {
  const w = input.ownerDocument.defaultView;
  input.dispatchEvent(new w.KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
}

function shownFrame() {
  return doc().querySelector("#query-frames > iframe.query-frame:not(.loading)");
}

async function waitForResults(query, msg) {
  await waitForCondition(() => {
    const shown = shownFrame();
    const url = shown?.contentWindow.location.href;
    return url && new URL(url).searchParams.get("q") == query &&
      shown.contentDocument.querySelector(".query-result");
  }, msg);
}

add_task(async function test_QueryGate() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const response = await win().fetch("/trydelta/query-terms", { cache: "no-store" });
  const gate = new (win().QueryGate)(await response.json());
  for (const [input, searchable, why] of [
    ["Wid", true, "Text of 3 characters is searched"],
    ["Wi", false, "but not less"],
    ["我们", true, "(as CJK characters count twice)"],
    ["calls-to:Widget", false, "A diagram's term waits for Enter (its identifier must be exact)"],
    ["'Widget", false, "as does an open quote"],
    ["'Widget::Frob' path:widget", true, "A qualified name and a path are searched"],
    ["Widget path:", false, "but not a term without its value"],
    ["Widget -", false, "nor a negated term's start"],
    ["Widget -path:gizmos", true, "A negated path is searched"],
    ["case:sens Widget", false, "A term's value must be one of its values"],
    ["case:sensitive Widget", true, "(and is searched once it is)"],
    ["re:Wid(", false, "A regexp waits for Enter"],
    ["try deadbeefdead", false, "Input a DWIM rule matches isn't searched"],
    [". https://bugzil.la/1517978", false, "(after the helper prefix too)"],
  ]) {
    is(gate.check(input).searchable, searchable, `${why}: ${JSON.stringify(input)}`);
  }
  is(win().QueryGate.delay(3), 500, "3 characters are searched 500 ms after the last keystroke");
  is(win().QueryGate.delay(6), 300, "and 6 or more after 300 ms");
});

add_task(async function test_QueryFramesTyping() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const input = doc().querySelector("#query");
  // (Kept unless the page reloads.)
  win().notReloaded = true;

  type(input, "Wi");
  await TestUtils.sleep(800);
  ok(!doc().querySelector("#query-frames"), "Too little text isn't searched");

  type(input, "dget");
  await waitForResults("Widget", "Typing a query shows its results");
  ok(doc().documentElement.classList.contains("query-frames-shown"), "over the page");
  is(win().getComputedStyle(doc().querySelector("#scrolling")).display, "none", "whose content is hidden");
  const results = shownFrame().contentDocument;
  ok(results.documentElement.classList.contains("in-query-frame"), "The results' page knows it's in a frame");
  is(results.defaultView.getComputedStyle(results.querySelector("#fixed-header")).display, "none",
     "and has no header of its own");
  is(win().location.pathname, "/trydelta/query/default", "The page's URL is the results'");
  // (By their states: history.length stops at 50, which the tests' tab
  // reaches.)
  const session = win().history.state?.queryFrames;
  ok(session, "in a history entry of the typing's");
  is(doc().title, results.title, "and so is its title");

  type(input, "X");
  await waitForResults("WidgetX", "Typing more shows the new results");
  is(win().history.state?.queryFrames, session, "in the same entry");
  is(doc().querySelectorAll("#query-frames > iframe").length, 1, "without the previous frame");

  type(input, " calls-to:Foo");
  await TestUtils.sleep(800);
  is(new URL(shownFrame().contentWindow.location.href).searchParams.get("q"), "WidgetX",
     "A query with a diagram's term isn't searched as it's typed");

  escape(input);
  await waitForCondition(() => win().location.pathname == "/trydelta/source/user.cpp",
                         "Escape goes back to the page");
  ok(!doc().querySelector("#query-frames"), "without the results");
  ok(!doc().documentElement.classList.contains("query-frames-shown"), "showing its content");
  ok(win().notReloaded, "which isn't reloaded");
  is(input.value, "", "with the box as it was");
});

add_task(async function test_QueryFramesBack() {
  await TestUtils.loadPath("/trydelta/query/default?q=Widget");
  const input = doc().querySelector("#query");
  win().notReloaded = true;
  type(input, "X");
  await waitForResults("WidgetX", "On a query's page, typing shows the new query's results");
  win().history.back();
  await waitForCondition(() => new URL(win().location.href).searchParams.get("q") == "Widget",
                         "Back goes back to the query's page");
  ok(!doc().querySelector("#query-frames"), "without the results");
  ok(win().notReloaded, "which isn't reloaded");
  is(input.value, "Widget", "with its query in the box");
});

add_task(async function test_QueryFramesEnter() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  let input = doc().querySelector("#query");
  type(input, "Widget");
  await waitForResults("Widget", "Typing shows the results");
  const url = shownFrame().contentWindow.location.href;

  let load = TestUtils.waitForLoad();
  input.form.requestSubmit();
  await load;
  is(win().location.href, url, "Enter makes the results the page");
  ok(!doc().documentElement.classList.contains("in-query-frame"), "not in a frame");
  // (In the typing's entry: see the last Escape's, below.)
  ok(win().history.state?.queryEntered, "which Escape goes back from");

  input = doc().querySelector("#query");
  win().notReloaded = true;
  type(input, "X");
  await waitForResults("WidgetX", "Typing after Enter shows results");
  ok(win().history.state?.queryFrames, "in another entry");
  escape(input);
  await waitForCondition(() => win().location.href == url, "whose Escape goes back to the page Enter made");
  ok(win().notReloaded, "(without reloading it)");

  // (Another page's, which may come back from the back-forward cache,
  // without a load.)
  escape(input);
  await waitForCondition(() => win().location.pathname == "/trydelta/source/user.cpp" &&
                         doc().readyState == "complete",
                         "where Escape goes back to the page typing started on");
});

add_task(async function test_QueryFramesLinks() {
  await TestUtils.loadPath("/trydelta/source/user.cpp");
  const input = doc().querySelector("#query");
  type(input, "Widget");
  await waitForResults("Widget", "Typing shows the results");
  const link = shownFrame().contentDocument.querySelector('.query-result a[href*="/source/widget"]');
  ok(link, "with links to files");
  const load = TestUtils.waitForLoad();
  link.click();
  await load;
  is(win().location.pathname, new URL(link.href).pathname, "which go to the whole page");
  ok(!doc().documentElement.classList.contains("in-query-frame"), "not to the frame");
});
