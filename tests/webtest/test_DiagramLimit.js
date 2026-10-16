"use strict";

// A calls-to diagram includes at most the caller limit's callers of a symbol
// (the distinct callable contexts of its uses; see `OverloadKind::UsesCallers`
// in the tools), which it says, with a button to lift the limit.  Each
// fixture's target has 20 callers: in one file, in two files, and in 20 files.
function shownCallers(symPrefix) {
  const re = new RegExp(`^${symPrefix}\\d+caller\\d+Ev$`);
  const syms = [...frame.contentDocument.querySelectorAll("[data-symbols]")]
    .map(node => node.getAttribute("data-symbols"))
    .filter(syms => re.test(syms));
  return new Set(syms).size;
}

async function checkCallerLimit(fixture, symPrefix) {
  await TestUtils.loadQuery("tests", `calls-to:'diagram::${fixture}::target' depth:4`);

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(warningBox,"warning is present");

    is(warningBox.querySelector(".diagram-limit-kind").textContent.trim(),
       "too many callers",
       "kind is shown");

    ok(warningBox.textContent.includes("hit the local limit"),
       "limit type is shown");
    ok(warningBox.textContent.includes("only 16 included"),
       "how many callers are included is shown");

    is(shownCallers(symPrefix), 16, "only that many callers are shown");

    const loadPromise = TestUtils.waitForLoad();
    const lift = warningBox.querySelector("li button");
    TestUtils.click(lift);
    await loadPromise;
  }

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(!warningBox, "warning is not present");

    is(shownCallers(symPrefix), 20, "all of the callers are shown");
  }
}

add_task(async function test_CallersInOneFile() {
  await TestUtils.resetFeatureGate("diagramming");
  await checkCallerLimit("uses_lines_local", "_ZN7diagram16uses_lines_local");
});

add_task(async function test_CallersInTwoFiles() {
  await checkCallerLimit("uses_lines_global", "_ZN7diagram17uses_lines_global");
});

add_task(async function test_CallersInManyFiles() {
  await checkCallerLimit("uses_paths", "_ZN7diagram10uses_paths");
});

add_task(async function test_FewCallersManyUses() {
  // (24 uses by 3 callers.)
  await TestUtils.loadQuery("tests", "calls-to:'diagram::few_callers::target' depth:4");

  ok(!frame.contentDocument.querySelector(".diagram-limit-warning"), "warning is not present");
  for (const n of [0, 1, 2]) {
    ok(frame.contentDocument.querySelector(`[data-symbols="_ZN7diagram11few_callers7caller${n}Ev"]`),
       `caller${n} is shown`);
  }
});

add_task(async function test_NodeLimit() {
  // ("path-limit" is the caller limit's old name.)
  await TestUtils.loadQuery("tests", "calls-to:'diagram::uses_paths::target' depth:4 path-limit:256 node-limit:16");

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(warningBox,"warning is present");

    is(warningBox.querySelector(".diagram-limit-kind").textContent.trim(),
       "too many nodes",
       "kind is shown");

    ok(warningBox.textContent.includes("hit the global limit"),
       "limit type is shown");

    const loadPromise = TestUtils.waitForLoad();
    const lift = warningBox.querySelector("li button");
    TestUtils.click(lift);
    await loadPromise;
  }

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(!warningBox, "warning is not present");

    const caller19 = frame.contentDocument.querySelector(`[data-symbols="_ZN7diagram10uses_paths8caller19Ev"]`);
    ok(caller19, "caller19 is shown");
  }
});
