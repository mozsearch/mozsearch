"use strict";

add_task(async function test_FieldLayoutColumns() {
  const sym = "field_layout::field_type::S";

  await TestUtils.loadPath(`/tests/query/default?q=field-layout:'${sym}'`);
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  is(query.value, "field-layout:'field_layout::field_type::S'",
     "Query for field layout is set");

  const nameCheckbox = frame.contentDocument.querySelector("#col-show-name");
  const typeCheckbox = frame.contentDocument.querySelector("#col-show-type");
  const lineCheckbox = frame.contentDocument.querySelector("#col-show-line");

  is(nameCheckbox.checked, true,
     "name checkbox is checked by default");
  is(typeCheckbox.checked, false,
     "type checkbox is not checked by default");
  is(lineCheckbox.checked, true,
     "line checkbox is checked by default");

  const nameCell = frame.contentDocument.querySelector(".name-cell");
  is(TestUtils.isShown(nameCell), true,
     "name cells are shown by default");
  const typeCell = frame.contentDocument.querySelector(".type-cell");
  is(TestUtils.isShown(typeCell), false,
     "type cells are hidden by default");
  const lineCell = frame.contentDocument.querySelector(".line-cell");
  is(TestUtils.isShown(lineCell), true,
     "line cells are shown by default");

  TestUtils.clickCheckbox(nameCheckbox);

  is(TestUtils.isShown(nameCell), false,
     "name cells are hidden");
  is(TestUtils.isShown(typeCell), false,
     "type cells are hidden");
  is(TestUtils.isShown(lineCell), true,
     "line cells are shown");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes(encodeURIComponent("hide-cols:name")),
    "URL is updated");

  is(query.value, "field-layout:'field_layout::field_type::S' hide-cols:name",
     "Query is updated");

  TestUtils.clickCheckbox(lineCheckbox);

  is(TestUtils.isShown(nameCell), false,
     "name cells are hidden");
  is(TestUtils.isShown(typeCell), false,
     "type cells are hidden");
  is(TestUtils.isShown(lineCell), false,
     "line cells are hidden");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes(encodeURIComponent("hide-cols:name,line")),
    "URL is updated");

  is(query.value, "field-layout:'field_layout::field_type::S' hide-cols:name,line",
     "Query is updated");

  TestUtils.clickCheckbox(typeCheckbox);

  is(TestUtils.isShown(nameCell), false,
     "name cells are hidden");
  is(TestUtils.isShown(typeCell), true,
     "type cells are shown");
  is(TestUtils.isShown(lineCell), false,
     "line cells are hidden");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes(encodeURIComponent("show-cols:type")) &&
      frame.contentDocument.location.href.includes(encodeURIComponent("hide-cols:name,line")),
    "URL is updated");

  is(query.value, "field-layout:'field_layout::field_type::S' show-cols:type hide-cols:name,line",
     "Query is updated");

  TestUtils.clickCheckbox(nameCheckbox);

  is(TestUtils.isShown(nameCell), true,
     "name cells are shown");
  is(TestUtils.isShown(typeCell), true,
     "type cells are shown");
  is(TestUtils.isShown(lineCell), false,
     "line cells are hidden");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes(encodeURIComponent("show-cols:type")) &&
      frame.contentDocument.location.href.includes(encodeURIComponent("hide-cols:line")),
    "URL is updated");

  is(query.value, "field-layout:'field_layout::field_type::S' show-cols:type hide-cols:line",
     "Query is updated");

  TestUtils.clickCheckbox(lineCheckbox);

  is(TestUtils.isShown(nameCell), true,
     "name cells are shown");
  is(TestUtils.isShown(typeCell), true,
     "type cells are shown");
  is(TestUtils.isShown(lineCell), true,
     "line cells are shown");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes(encodeURIComponent("show-cols:type")) &&
      !frame.contentDocument.location.href.includes(encodeURIComponent("hide-cols:")),
    "URL is updated");

  is(query.value, "field-layout:'field_layout::field_type::S' show-cols:type",
     "Query is updated");

  TestUtils.clickCheckbox(typeCheckbox);

  is(TestUtils.isShown(nameCell), true,
     "name cells are shown");
  is(TestUtils.isShown(typeCell), false,
     "type cells are hidden");
  is(TestUtils.isShown(lineCell), true,
     "line cells are shown");

  await waitForCondition(
    () => !frame.contentDocument.location.href.includes(encodeURIComponent("show-cols:")) &&
      !frame.contentDocument.location.href.includes(encodeURIComponent("hide-cols:")),
    "URL is updated");

  is(query.value, "field-layout:'field_layout::field_type::S'",
     "Query is updated");
});
