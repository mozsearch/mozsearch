"use strict";

// Diffs' (and stack contexts') "Changes" section in the panel (see
// change-nav.js): the previous and next change ("k" and "j"), at a
// granularity of runs of changed rows, paragraphs or sections, remembered.
// 75472c69 changed coverage-history.js in 4 hunks.
add_task(async function test_ChangeNav() {
  const path = "/searchfox/diff/75472c69eac5ea435156d0e84012fbdec128fbb9/static/js/coverage-history.js";
  await TestUtils.loadPath(path);
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("change-nav-granularity"));
  await TestUtils.loadPath(path);

  let doc = frame.contentDocument;
  const section = doc.querySelector(".panel .change-nav");
  ok(section, "The diff's panel has a Changes section");
  const status = () => doc.querySelector(".change-nav-status").textContent;
  const granularity = key => TestUtils.click(doc.querySelector(`.change-nav-granularity button[data-granularity="${key}"]`));
  const count = () => parseInt(status().match(/(\d+) \w+$/)[1], 10);
  is(doc.querySelector('.change-nav-granularity button[aria-pressed="true"]').dataset.granularity, "para",
     "Paragraphs by default");

  granularity("token");
  const tokens = count();
  granularity("para");
  const paras = count();
  granularity("section");
  const sections = count();
  ok(tokens >= paras && paras >= sections && sections >= 1,
     `Coarser granularities have no more units (${tokens} runs, ${paras} paragraphs, ${sections} sections)`);

  granularity("token");
  const key = k => doc.documentElement.dispatchEvent(new frame.contentWindow.KeyboardEvent("keypress", { key: k, bubbles: true }));
  key("j");
  is(status(), `1 of ${tokens} change${tokens == 1 ? "" : "s"}`, "j goes to the first change");
  const marked = [...doc.querySelectorAll(".change-nav-current")];
  ok(marked.length > 0 && marked.every(row => row.querySelector("code.plus-line, code.minus-line")),
     "and marks its changed rows");
  if (tokens > 1) {
    key("j");
    ok(status().startsWith("2 of"), "j again goes to the next one");
    key("k");
    ok(status().startsWith("1 of"), "and k back");
  }

  await TestUtils.loadPath(path);
  doc = frame.contentDocument;
  is(doc.querySelector('.change-nav-granularity button[aria-pressed="true"]').dataset.granularity, "token",
     "The granularity is remembered");
  frame.contentWindow.localStorage.removeItem(frame.contentWindow.storageKey("change-nav-granularity"));
});
