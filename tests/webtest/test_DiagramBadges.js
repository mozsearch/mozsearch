"use strict";

async function waitForTooltip(label) {
  await waitForCondition(() => {
    let tooltip = frame.contentDocument.querySelector(".diagram-badge-tooltip");
    return tooltip && tooltip.textContent == label;
  }, "tooltip is added");
}

async function waitForNoTooltip() {
  await waitForCondition(() => {
    let tooltip = frame.contentDocument.querySelector(".diagram-badge-tooltip");
    return !tooltip;
  }, "tooltip is removed");
}

add_task(async function test_DiagramBadges_basic() {
  await TestUtils.loadQuery("tests", "class-diagram:'diagram_badges::C2' depth:4");

  const texts = [...frame.contentDocument.querySelectorAll(`svg text[text-decoration="underline"]`)];

  const wc = texts.find(t => t.textContent.startsWith("WC"));
  ok(wc, "WC badge exists");

  TestUtils.dispatchMouseEvent("mouseenter", wc);
  await waitForTooltip("nsWrapperCache");
  TestUtils.dispatchMouseEvent("mouseleave", wc);
  await waitForNoTooltip();

  const strongRef = texts.find(t => t.textContent.startsWith("\u{1f4aa}"));
  ok(strongRef, "StrongRef badge exists");

  TestUtils.dispatchMouseEvent("mouseenter", strongRef);
  await waitForTooltip("Strong pointer");
  TestUtils.dispatchMouseEvent("mouseleave", strongRef);
  await waitForNoTooltip();

  const refCount = texts.find(t => t.textContent.startsWith("\u{1f9ee}"));
  ok(refCount, "RefCount badge exists");

  TestUtils.dispatchMouseEvent("mouseenter", refCount);
  await waitForTooltip("Reference counted class");
  TestUtils.dispatchMouseEvent("mouseleave", refCount);
  await waitForNoTooltip();
});

add_task(async function test_DiagramBadges_legend() {
  await TestUtils.loadQuery("tests", "class-diagram:'diagram_badges::C2' depth:4");

  const toggle = frame.contentDocument.querySelector("#diagram-panel-toggle");
  TestUtils.click(toggle);

  const panel = frame.contentDocument.querySelector("#diagram-panel");
  await waitForCondition(() => !panel.classList.contains("hidden"),
                         "Panel is shown");

  await waitForCondition(() => panel.querySelector("button"),
                         "Apply button is shown");

  const legend = frame.contentDocument.querySelector("#diagram-legend-pane");
  ok(legend.textContent.includes("\u{1f4aa}"),
     "Strong Pointer badge exists");
  ok(legend.textContent.includes("Strong pointer"),
     "Strong Pointer description exists");
});
