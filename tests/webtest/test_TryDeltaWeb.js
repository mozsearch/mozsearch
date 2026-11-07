"use strict";
// webtest-group: trydelta-push (they import it)

// The trydelta tree's try push 0123456789ab (see test_TryDelta.js) renames a
// web-platform-test page's inline script's function and its stylesheet's
// custom property, whose analyses are the tree's (its JS environments', its
// markup's, its stylesheet's and its links'; see scripts/try-analyze-web.sh).

const PUSH = "0123456789ab";

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

// A web-platform-test page's inline script and stylesheet the push changes
// are analyzed as the tree's are: the page's script in its JS environment,
// with scip-typescript, its event handler's call of the script's renamed
// function too, its style's use of the stylesheet's renamed custom property
// of the stylesheet's definition (each file's custom properties its own), and
// its link to the stylesheet a use of its file.
add_task(async function test_TryDeltaWebAnalysis() {
  const head = await importPush();
  const page = "testing/web-platform/tests/greeting/greeting.html";
  const symbolsOn = (line, text) =>
    [...doc().querySelectorAll(`#line-${line} span[data-symbols]`)]
      .filter(span => span.textContent == text)
      .flatMap(span => span.dataset.symbols.split(","));

  await TestUtils.loadPath(`/trydelta/source/${page}`);
  ok(symbolsOn(10, "greetVisitor").some(s => s.endsWith(`${page}/greetVisitor().`)),
     "The indexed revision's page's inline script is analyzed in its environment");
  await TestUtils.loadPath(`/trydelta/rev/${head}/${page}`);
  ok(symbolsOn(10, "welcomeVisitor").some(s => s.endsWith(`${page}/welcomeVisitor().`)),
     "and so is the push's, with its function's new name");
  ok(symbolsOn(8, "welcomeVisitor").some(s => s.endsWith(`${page}/welcomeVisitor().`)),
     "which its event handler calls");
  ok(symbolsOn(14, "test").some(s => s.endsWith("testing/web-platform/tests/resources/testharness.js/test().")),
     "and its test() is its harness's");
  ok(symbolsOn(6, "--greeting-space").includes(
       "CSSPROP_testing/web@2Dplatform/tests/greeting/support/greeting@2Ecss/@2D@2Dgreeting@2Dspace"),
     "Its style's use of its stylesheet's renamed custom property is of the stylesheet's definition");
  ok(symbolsOn(4, "support/greeting.css").includes(
       "FILE_testing/web@2Dplatform/tests/greeting/support/greeting@2Ecss"),
     "and its link to the stylesheet is a use of its file");

  await TestUtils.loadPath(`/trydelta/rev/${head}/testing/web-platform/tests/greeting/support/greeting.css`);
  ok(symbolsOn(3, "--greeting-space").includes(
       "CSSPROP_testing/web@2Dplatform/tests/greeting/support/greeting@2Ecss/@2D@2Dgreeting@2Dspace"),
     "The push's stylesheet's custom property is its own");
});
