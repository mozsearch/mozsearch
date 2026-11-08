"use strict";
// webtest-group: trydelta-push (they import it)
// webtest-timeout: 120 (it imports a push, with its semantic analysis, and
// waits for its pages, as test_TryDelta.js's tasks do)

// The trydelta tree's try push 246813579bdf adds a gadget's WebIDL, IPDL,
// Fluent strings, test manifest, moz.build file and a C++ use of a pref,
// whose analyses are the tree's (see scripts/try-analyze-rest.sh), and
// changes none of the tree's C++, whose analysis, with the tree's GC data
// (see tests/trydelta/setup), is the tree's.

const PUSH = "246813579bdf";

function doc() {
  return frame.contentDocument;
}

// Import the push (if it isn't yet), and return the head of its stack rebased
// onto the indexed revision (as test_TryDelta.js's `importPush` does).
async function importPush() {
  await TestUtils.loadPath(`/trydelta/try/hg/${PUSH}`);
  if (!doc().querySelector(".explore-page")) {
    doc().querySelector(".try-import-button").click();
    await waitForCondition(() => doc()?.querySelector(".explore-page"),
                           "The push is imported", 500, 240);
  }
  const link = doc().querySelector(".explore-semantic-head a")?.getAttribute("href");
  ok(link?.startsWith("/trydelta/rev/"), "The push's page links to its semantic analysis");
  return link.split("/")[3];
}

// The symbols of the line's tokens whose text is `text`.
function symbolsOn(line, text) {
  return [...doc().querySelectorAll(`#line-${line} span[data-symbols]`)]
    .filter(span => span.textContent.includes(text))
    .flatMap(span => span.dataset.symbols.split(","));
}

// The context menu of the line's token with `sym`.
async function menuOf(line, sym) {
  const span = [...doc().querySelectorAll(`#line-${line} span[data-symbols]`)]
    .find(span => span.dataset.symbols.split(",").includes(sym));
  ok(span, `${sym} is on line ${line}`);
  TestUtils.click(span);
  const menu = doc().querySelector("#context-menu");
  await waitForShown(menu, "Context menu is shown");
  return menu;
}

add_task(async function test_TryDeltaRestAnalysis() {
  await TestUtils.resetFeatureGate("semanticInfo");
  const head = await importPush();
  const rev = path => TestUtils.loadPath(`/trydelta/rev/${head}/${path}`);

  // Its WebIDL's, with its symbols' spec definitions and MDN pages (as the
  // tree's spec-analyze adds them; see build-try-delta's `cite_specs`).
  await rev("gadget.webidl");
  const links = [...(await menuOf(5, "WEBIDL_Gadget_poke")).querySelectorAll("a")];
  ok(links.find(a => a.textContent.includes("IDL spec method"))?.textContent
       .includes("examplebs Gadget/poke"),
     "The push's WebIDL method goes to the spec's definition");
  ok(links.some(a => a.getAttribute("href") == "https://developer.mozilla.org/docs/Web/API/Gadget/poke"),
     "and to its MDN page");

  await rev("PGadget.ipdl");
  ok(symbolsOn(7, "aAmount").includes("IPC_mozilla_trydelta_PGadget_Poke__aAmount"),
     "Its IPDL's message's parameter is analyzed");

  await rev("gadget.ftl");
  ok(symbolsOn(2, "gadget-label").includes("FTL_gadget-label"),
     "Its Fluent message's reference is of the message it defines");

  await rev("gadget.toml");
  ok(symbolsOn(2, "gadget.h").includes("FILE_gadget@2Eh"),
     "Its test manifest's support file is a use of the file");
  ok(symbolsOn(3, "gadget.loudness").includes("PREFS_gadget@2Eloudness"),
     "and its pref, of the pref");

  await rev("moz.build");
  ok(symbolsOn(7, "gadget_prefs.cpp").includes("FILE_gadget_prefs@2Ecpp"),
     "Its moz.build file's source is a use of the file");

  // Its C++'s use of the pref, which the semantic analysis adds to its
  // compilation's analysis.
  await rev("gadget_prefs.cpp");
  ok(symbolsOn(11, "gadget.loudness").includes("PREFS_gadget@2Eloudness"),
     "Its C++'s use of a pref is of the pref");
  ok(symbolsOn(11, "GadgetIsLoud").length, "beside its compilation's records");

  // A function of the tree's which can GC still can, as its analysis is the
  // tree's (with its GC data, which the push's compilation's analysis of the
  // file gets too).
  await rev("user.cpp");
  const gc = [...(await menuOf(6, "_ZN8trydelta9UseWidgetEv")).querySelectorAll(".contextmenu-row")]
    .find(row => row.textContent.match("Can GC|Cannot GC"));
  is(gc?.textContent, "Can GC", "The tree's function's GC is the tree's");

  // And none of its files is without analysis.
  const response = await fetch(`/trydelta/rev-canvas/${head}/canvas.json`, { cache: "no-store" });
  const others = (await response.json()).other_files;
  is(others.filter(file => file.note == "not analyzed").map(file => file.path).join(", "), "",
     "None of the push's files is without analysis");
});
