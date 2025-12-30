"use strict";

function roundToOneDecimalPlace(n) {
  return Math.round(n * 10) / 10;
}

add_task(async function test_SearchBoxLayout_noRevision() {
  await TestUtils.loadPath("/tests/source/");

  const searchbox = frame.contentDocument.querySelector("#search-box");
  const panel = frame.contentDocument.querySelector("#panel");

  is(searchbox.getBoundingClientRect().bottom,
     panel.getBoundingClientRect().top,
     "the navigation panel should touch the bottom of the search box");

  const option = searchbox.querySelector("#option-section");
  const caseCheckbox = option.querySelector("#case");
  const caseLabel = caseCheckbox.closest("label");

  is(caseCheckbox.getBoundingClientRect().height, 14,
     "Checkbox should have fixed height");
  is(window.getComputedStyle(caseCheckbox).marginTop, "3px",
     "Checkbox should have fixed margin top");
  is(window.getComputedStyle(caseCheckbox).marginBottom, "3px",
     "Checkbox should have fixed margin bottom");

  is(caseLabel.getBoundingClientRect().height,
     caseCheckbox.getBoundingClientRect().height + 6,
     "Checkbox label shouldn't have any extra height");

  is(option.getBoundingClientRect().height,
     caseLabel.getBoundingClientRect().height * 2,
     "The option section should have exactly twice height as the checkbox");

  let searchBoxPadding = window.getComputedStyle(searchbox).paddingTop;
  is(window.getComputedStyle(searchbox).paddingBottom,
     searchBoxPadding,
     "Search box should have same padding for top vs bottom");
  is(window.getComputedStyle(searchbox).borderBottomWidth, "1px",
     "Search box should have fixed border bottom width");
  is(roundToOneDecimalPlace(searchbox.getBoundingClientRect().height),
     roundToOneDecimalPlace(parseFloat(searchBoxPadding) * 2
                                       + option.getBoundingClientRect().height
                                       + 1),
     "Search box should have pre-defined height");
});

add_task(async function test_SearchBoxLayout_withRevision() {
  await TestUtils.loadPath("/searchfox/source/Makefile");

  const searchbox = frame.contentDocument.querySelector("#search-box");
  const panel = frame.contentDocument.querySelector("#panel");

  is(searchbox.getBoundingClientRect().bottom,
     panel.getBoundingClientRect().top,
     "the navigation panel should touch the bottom of the search box");

  const option = searchbox.querySelector("#option-section");
  const caseCheckbox = option.querySelector("#case");
  const caseLabel = caseCheckbox.closest("label");
  const revision = searchbox.querySelector("#revision");

  is(caseCheckbox.getBoundingClientRect().height, 14,
     "Checkbox should have fixed height");
  is(window.getComputedStyle(caseCheckbox).marginTop, "3px",
     "Checkbox should have fixed margin top");
  is(window.getComputedStyle(caseCheckbox).marginBottom, "3px",
     "Checkbox should have fixed margin bottom");

  is(caseLabel.getBoundingClientRect().height,
     caseCheckbox.getBoundingClientRect().height + 6,
     "Checkbox label shouldn't have any extra height");

  is(option.getBoundingClientRect().height,
     caseLabel.getBoundingClientRect().height * 2,
     "The option section should have exactly twice height as the checkbox");

  let searchBoxPadding = parseFloat(window.getComputedStyle(searchbox).paddingTop);
  is(parseFloat(window.getComputedStyle(searchbox).paddingBottom),
     searchBoxPadding / 2,
     "Search box should have a half padding for top vs bottom");
  is(window.getComputedStyle(searchbox).borderBottomWidth, "1px",
     "Search box should have fixed border bottom width");
  is(roundToOneDecimalPlace(searchbox.getBoundingClientRect().height),
     roundToOneDecimalPlace(parseFloat(searchBoxPadding) * 1.5
                            + option.getBoundingClientRect().height
                            + revision.getBoundingClientRect().height
                            + 1),
     "Search box should have pre-defined height");
});

add_task(async function test_SearchBoxLayout_Query() {
  await TestUtils.loadPath("/tests/source/");

  const bottom = frame.contentDocument.querySelector("#search-box").getBoundingClientRect().bottom;

  const sym = "field_layout::field_type::S";
  await TestUtils.loadQuery("tests", `field-layout:'${sym}'`);

  const searchbox = frame.contentDocument.querySelector("#search-box");
  const panel = frame.contentDocument.querySelector("#panel");

  is(searchbox.getBoundingClientRect().bottom,
     panel.getBoundingClientRect().top,
     "the navigation panel should touch the bottom of the search box");
  is(searchbox.getBoundingClientRect().bottom,
     bottom,
     "the search box should have the same height as source page");
});
