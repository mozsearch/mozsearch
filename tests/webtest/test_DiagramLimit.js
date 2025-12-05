"use strict";

add_task(async function test_UsesLinesLocal() {
  await TestUtils.loadPath("/tests/query/default?q=calls-to%3A%27diagram%3A%3Auses_lines_local%3A%3Atarget%27%20depth%3A4");

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(warningBox,"warning is present");

    is(warningBox.querySelector(".diagram-limit-kind").textContent.trim(),
       "too many lines",
       "kind is shown");

    ok(warningBox.textContent.includes("hit the local limit"),
       "limit type is shown");

    const loadPromise = TestUtils.waitForLoad();
    const lift = warningBox.querySelector("button");
    TestUtils.click(lift);
    await loadPromise;
  }

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(!warningBox, "warning is not present");

    const caller19 = frame.contentDocument.querySelector(`[data-symbols="_ZN7diagram16uses_lines_local8caller19Ev"]`);
    ok(caller19, "caller19 is shown");
  }
});

add_task(async function test_UsesLinesGlobal() {
  await TestUtils.loadPath("/tests/query/default?q=calls-to%3A%27diagram%3A%3Auses_lines_global%3A%3Atarget%27%20depth%3A4");

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(warningBox,"warning is present");

    is(warningBox.querySelector(".diagram-limit-kind").textContent.trim(),
       "too many lines",
       "kind is shown");

    ok(warningBox.textContent.includes("hit the global limit"),
       "limit type is shown");

    const loadPromise = TestUtils.waitForLoad();
    const lift = warningBox.querySelector("button");
    TestUtils.click(lift);
    await loadPromise;
  }

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(!warningBox, "warning is not present");

    const caller19 = frame.contentDocument.querySelector(`[data-symbols="_ZN7diagram17uses_lines_global8caller19Ev"]`);
    ok(caller19, "caller19 is shown");
  }
});

add_task(async function test_UsesPaths() {
  await TestUtils.loadPath("/tests/query/default?q=calls-to%3A%27diagram%3A%3Auses_paths%3A%3Atarget%27%20depth%3A4");

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(warningBox,"warning is present");

    is(warningBox.querySelector(".diagram-limit-kind").textContent.trim(),
       "too many uses",
       "kind is shown");

    ok(warningBox.textContent.includes("hit the local limit"),
       "limit type is shown");

    const loadPromise = TestUtils.waitForLoad();
    const lift = warningBox.querySelector("button");
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

add_task(async function test_NodeLimit() {
  await TestUtils.loadPath("/tests/query/default?q=calls-to%3A'diagram%3A%3Auses_paths%3A%3Atarget'+depth%3A4+path-limit%3A256+node-limit%3A16");

  {
    const warningBox = frame.contentDocument.querySelector(".diagram-limit-warning");
    ok(warningBox,"warning is present");

    is(warningBox.querySelector(".diagram-limit-kind").textContent.trim(),
       "too many nodes",
       "kind is shown");

    ok(warningBox.textContent.includes("hit the global limit"),
       "limit type is shown");

    const loadPromise = TestUtils.waitForLoad();
    const lift = warningBox.querySelector("button");
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
