"use strict";

add_task(async function test_yaml() {
  await TestUtils.loadPath("/tests/source/yaml/nesting.yaml");

  const line = frame.contentDocument.querySelector(`#line-22`);
  frame.contentDocument.documentElement.scrollTop = line.getBoundingClientRect().top;

  await waitForCondition(() => frame.contentDocument.querySelector(`#line-1`).classList.contains("stuck"),
                         "1st nesting should be stuck");
  ok(frame.contentDocument.querySelector(`#line-1`).parentNode.classList.contains("nesting-depth-0"),
     "1st nesting has nesting container");
  ok(frame.contentDocument.querySelector(`#line-6`).classList.contains("stuck"),
     "2nd nesting should be stuck");
  ok(frame.contentDocument.querySelector(`#line-6`).parentNode.classList.contains("nesting-depth-1"),
     "2nd nesting has nesting container");
  ok(frame.contentDocument.querySelector(`#line-9`).classList.contains("stuck"),
     "3rd nesting should be stuck");
  ok(frame.contentDocument.querySelector(`#line-9`).parentNode.classList.contains("nesting-depth-2"),
     "3rd nesting has nesting container");
  ok(frame.contentDocument.querySelector(`#line-11`).classList.contains("stuck"),
     "4th nesting should be stuck");
  ok(frame.contentDocument.querySelector(`#line-11`).parentNode.classList.contains("nesting-depth-3"),
     "4th nesting has nesting container");
  ok(frame.contentDocument.querySelector(`#line-12`).classList.contains("stuck"),
     "5th nesting should be stuck");
  ok(frame.contentDocument.querySelector(`#line-12`).parentNode.classList.contains("nesting-depth-4"),
     "5th nesting has nesting container");
});
