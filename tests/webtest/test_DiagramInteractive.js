"use strict";

add_task(async function test_DiagramInteractive_Visibility() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");

  const nodes = [
    "_ZN14diagram_ignore2F1Ev",
    "_ZN14diagram_ignore2F2Ev",
    "_ZN14diagram_ignore2F3Ev",
    "_ZN14diagram_ignore2F4Ev",
    "_ZN14diagram_ignore2F5Ev",
    "_ZN14diagram_ignore2F6Ev",
    "_ZN14diagram_ignore2F7Ev",
    "_ZN14diagram_ignore2F8Ev",
    "_ZN14diagram_ignore2F9Ev",
    "_ZN14diagram_ignore3F10Ev",
  ];
  const edges = [
    "_ZN14diagram_ignore2F2Ev->_ZN14diagram_ignore2F1Ev",
    "_ZN14diagram_ignore2F3Ev->_ZN14diagram_ignore2F2Ev",
    "_ZN14diagram_ignore2F4Ev->_ZN14diagram_ignore2F1Ev",
    "_ZN14diagram_ignore2F5Ev->_ZN14diagram_ignore2F3Ev",
    "_ZN14diagram_ignore2F5Ev->_ZN14diagram_ignore2F4Ev",
    "_ZN14diagram_ignore2F6Ev->_ZN14diagram_ignore2F5Ev",
    "_ZN14diagram_ignore2F7Ev->_ZN14diagram_ignore2F6Ev",
    "_ZN14diagram_ignore2F8Ev->_ZN14diagram_ignore2F2Ev",
    "_ZN14diagram_ignore2F9Ev->_ZN14diagram_ignore2F7Ev",
    "_ZN14diagram_ignore2F9Ev->_ZN14diagram_ignore2F8Ev",
    "_ZN14diagram_ignore3F10Ev->_ZN14diagram_ignore2F9Ev"
  ];

  function assertFullyShown(rect, name) {
    const documentWidth = frame.contentDocument.documentElement.scrollWidth;
    const documentHeight = frame.contentDocument.documentElement.scrollHeight;

    const nodeRect = `(${rect.left}, ${rect.top})-(${rect.right}, ${rect.bottom})`;
    const documentRect = `(0, 0)-(${documentWidth}, ${documentHeight})`;
    const details = `${nodeRect} vs ${documentRect}`;
    const message = `${name} is fully shown : ${details}`;

    ok(rect.left > 0, `left: ${message}`);
    ok(rect.top > 0, `top: ${message}`);
    ok(rect.right < documentWidth, `right: ${message}`);
    ok(rect.bottom < documentHeight, `bottom: ${message}`);
  }

  for (const id of nodes) {
    const node = frame.contentDocument.querySelector(`[data-symbols="${id}"]`);
    ok(!!node, `${id} node exists`);

    const rect = node.getBoundingClientRect();
    assertFullyShown(rect, `${id} node`);
  }

  for (const id of edges) {
    const edge = frame.contentDocument.querySelector(`[data-symbols="${id}"]`);
    ok(!!edge, `${id} edge exists`);

    const rect = edge.getBoundingClientRect();
    assertFullyShown(rect, `${id} edge`);
  }
});

add_task(async function test_DiagramInteractive_ZoomButtons() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");

  const zoomIn = frame.contentDocument.querySelector(".interactive-graph-button-zoom-in");
  const zoomOut = frame.contentDocument.querySelector(".interactive-graph-button-zoom-out");
  const fit = frame.contentDocument.querySelector(".interactive-graph-button-fit");

  const node = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
  const initRect = node.getBoundingClientRect();

  TestUtils.click(zoomIn);

  await waitForCondition(() => {
    const rect = node.getBoundingClientRect();
    return rect.width >= initRect.width * (1.5 - 0.1);
  }, "After zoom-in, the node should be shown as 1.5x");

  TestUtils.click(zoomIn);

  await waitForCondition(() => {
    const rect = node.getBoundingClientRect();
    return rect.width >= initRect.width * (2.25 - 0.1);
  }, "After zoom-in again, the node should be shown as 2.25x");

  TestUtils.click(zoomOut);

  await waitForCondition(() => {
    const rect = node.getBoundingClientRect();
    return rect.width <= initRect.width * (1.5 + 0.1);
  }, "After zoom-out, the node should be shown as 1.5x");

  TestUtils.click(fit);
  await waitForCondition(() => {
    const rect = node.getBoundingClientRect();
    return Math.round(rect.width) == Math.round(initRect.width);
  }, "After fit, the node should be shown as the initial size");
});

add_task(async function test_DiagramInteractive_pan() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");

  const node = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
  const initRect = node.getBoundingClientRect();

  const viewport = frame.contentDocument.querySelector("#interactive-graph-viewport");
  const pointerIds = TestUtils.spyPointerCapture(viewport);

  TestUtils.dispatchPointerEvent("pointerdown", viewport, {
    button: 0,
    pointerId: 1,
    clientX: 100,
    clientY: 100,
  });

  ok(pointerIds.has(1), "the pointer is captured");

  TestUtils.dispatchPointerEvent("pointermove", viewport, {
    button: 0,
    pointerId: 1,
    clientX: 200,
    clientY: 100,
  });

  ok(pointerIds.has(1), "the pointer is captured");

  TestUtils.dispatchPointerEvent("pointerup", viewport, {
    button: 0,
    pointerId: 1,
    clientX: 200,
    clientY: 100,
  });

  ok(!pointerIds.has(1), "the pointer is no longer captured");

  const rect1 = node.getBoundingClientRect();

  ok(rect1.left > initRect.left, "The node is shifted to right");
  ok(rect1.top == initRect.top, "The node is not shifted vertically");

  TestUtils.dispatchPointerEvent("pointerdown", viewport, {
    button: 0,
    pointerId: 1,
    clientX: 100,
    clientY: 100,
  });

  ok(pointerIds.has(1), "the pointer is captured");

  TestUtils.dispatchPointerEvent("pointermove", viewport, {
    button: 0,
    pointerId: 1,
    clientX: 100,
    clientY: 200,
  });

  ok(pointerIds.has(1), "the pointer is captured");

  TestUtils.dispatchPointerEvent("pointerup", viewport, {
    button: 0,
    pointerId: 1,
    clientX: 100,
    clientY: 200,
  });

  ok(!pointerIds.has(1), "the pointer is no longer captured");

  const rect2 = node.getBoundingClientRect();

  ok(rect2.left == rect1.left, "The node is not shifted horizontally");
  ok(rect2.top > rect1.top, "The node is shifted to bottom");
});

add_task(async function test_DiagramInteractive_wheel_zoom_scroll() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");

  const node = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
  const initRect = node.getBoundingClientRect();

  const viewport = frame.contentDocument.querySelector("#interactive-graph-viewport");

  TestUtils.dispatchWheelEvent("wheel", viewport, {
    deltaX: -100,
    deltaY: 0,
    clientX: 200,
    clientY: 200,
  });

  const rect1 = node.getBoundingClientRect();

  ok(rect1.left > initRect.left, "The node is shifted to right");
  ok(rect1.top == initRect.top, "The node is not shifted vertically");
  ok(rect1.width == initRect.width, "The node should be in the same size");

  TestUtils.dispatchWheelEvent("wheel", viewport, {
    deltaX: 0,
    deltaY: -100,
    clientX: 200,
    clientY: 200,
  });

  const rect2 = node.getBoundingClientRect();
  ok(rect2.left == rect1.left, "The node is not shifted horizontally");
  ok(rect2.top > rect1.top, "The node is shifted to bottom");
});

add_task(async function test_DiagramInteractive_wheel_zoom_out() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");

  const node = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
  const initRect = node.getBoundingClientRect();

  const viewport = frame.contentDocument.querySelector("#interactive-graph-viewport");

  TestUtils.dispatchWheelEvent("wheel", viewport, {
    ctrlKey: true,
    deltaY: 100,
    clientX: 200,
    clientY: 200,
  });

  const rect = node.getBoundingClientRect();

  ok(rect.width < initRect.width, "The node should be shown smaller");
});

add_task(async function test_DiagramInteractive_wheel_zoom_in() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'diagram_ignore::F1' depth:8 graph-format:mozsearch-interactive");

  const node = frame.contentDocument.querySelector(`[data-symbols="_ZN14diagram_ignore2F1Ev"]`);
  const initRect = node.getBoundingClientRect();

  const viewport = frame.contentDocument.querySelector("#interactive-graph-viewport");

  TestUtils.dispatchWheelEvent("wheel", viewport, {
    ctrlKey: true,
    deltaY: -100,
    clientX: 200,
    clientY: 200,
  });

  const rect = node.getBoundingClientRect();

  ok(rect.width > initRect.width, "The node should be shown smaller");
});
