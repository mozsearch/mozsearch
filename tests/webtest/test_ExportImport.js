"use strict";

add_task(async function test_ExportImport() {
  await TestUtils.loadPath("/tests/pages/settings.html");

  const status = frame.contentDocument.querySelector("#export-import-status");
  status.scrollIntoView();

  // Export to the clipboard

  const clipboard = TestUtils.spyClipboard();

  TestUtils.click(frame.contentDocument.querySelector("#export-clipboard"));
  await waitForCondition(
    () => status.textContent === "Exported to the clipboard.",
    "Status is updated");

  ok(clipboard.value.startsWith("{"),
     "JSON is copied");
  const data = JSON.parse(clipboard.value);
  ok("version" in data, "copied JSON has version");
  ok("settings" in data, "copied JSON has settings");

  // Export to the textarea

  const textarea = frame.contentDocument.querySelector("#export-import-json");

  TestUtils.click(frame.contentDocument.querySelector("#export-textarea"));
  await waitForCondition(
    () => status.textContent === "Exported to the textarea.",
    "Status is updated");

  is(clipboard.value, textarea.value,
     "Textarea is filled with the exported JSON");

  // Import from the clipboard

  data.settings.debug.ui = true;
  textarea.value = JSON.stringify(data);

  TestUtils.click(frame.contentDocument.querySelector("#import-textarea"));
  await waitForCondition(
    () => status.textContent.includes("Imported from the textarea."),
    "Status is updated");

  const debugUI = frame.contentDocument.querySelector("#debug--ui");
  is(debugUI.checked, true,
     "Setting is reflected to the checkbox");

  data.settings.debug.ui = false;
  textarea.value = JSON.stringify(data);
  TestUtils.click(frame.contentDocument.querySelector("#import-textarea"));

  is(debugUI.checked, false,
     "Setting is reflected to the checkbox again");

  // Undo import

  TestUtils.click(frame.contentDocument.querySelector("#undo-import"));
  await waitForCondition(
    () => status.textContent.includes("Restored to the settings"),
    "Status is updated");

  is(debugUI.checked, true,
     "Setting is restored");
});

add_task(async function test_ExportImport_Invalid() {
  await TestUtils.loadPath("/tests/pages/settings.html");

  const status = frame.contentDocument.querySelector("#export-import-status");
  status.scrollIntoView();

  const textarea = frame.contentDocument.querySelector("#export-import-json");
  textarea.value = "";

  TestUtils.click(frame.contentDocument.querySelector("#import-textarea"));
  await waitForCondition(
    () => status.textContent.includes("Failed to import:"),
    "The initial empty value should fail");

  status.textContent = "";

  textarea.value = "{";

  TestUtils.click(frame.contentDocument.querySelector("#import-textarea"));
  await waitForCondition(
    () => status.textContent.includes("Failed to import:"),
    "Syntax error should fail");

  status.textContent = "";

  textarea.value = "{}";

  TestUtils.click(frame.contentDocument.querySelector("#import-textarea"));
  await waitForCondition(
    () => status.textContent.includes("Failed to import:"),
    "Incomplete object should fail");
});
