"use strict";

add_task(async function test_Search() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "SimpleSearch");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("class SimpleSearch"),
    "1 class matches");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("SimpleSearch"),
    "URL is updated");
});

add_task(async function test_SearchCase() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "CaseSensitiveness");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Core code (2 lines") &&
      content.textContent.includes("class CaseSensitiveness1") &&
      content.textContent.includes("class casesensitiveness2"),
    "2 classes match with case==false");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("CaseSensitiveness") &&
      frame.contentDocument.location.href.includes("case=false"),
    "URL is updated");

  const caseCheckbox = frame.contentDocument.querySelector("#case");
  is(caseCheckbox.checked, false, "case checkbox is unchecked by default");

  TestUtils.clickCheckbox(caseCheckbox);

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("class CaseSensitiveness1") &&
      !content.textContent.includes("class casesensitiveness2"),
    "1 class matches with case==true");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("CaseSensitiveness") &&
      frame.contentDocument.location.href.includes("case=true"),
    "URL is updated");

  TestUtils.clickCheckbox(caseCheckbox);

  await waitForCondition(
    () => content.textContent.includes("Core code (2 lines") &&
      content.textContent.includes("class CaseSensitiveness1") &&
      content.textContent.includes("class casesensitiveness2"),
    "2 classes match with case==false");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("CaseSensitiveness") &&
      frame.contentDocument.location.href.includes("case=false"),
    "URL is updated");
});

add_task(async function test_SearchRegExp() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "Simpl.Search");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("No results for current query"),
    "Nothing matches with regexp==false");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("Simpl.Search") &&
      frame.contentDocument.location.href.includes("regexp=false"),
    "URL is updated");

  const regExpCheckbox = frame.contentDocument.querySelector("#regexp");
  is(regExpCheckbox.checked, false, "regexp checkbox is unchecked by default");

  TestUtils.clickCheckbox(regExpCheckbox);

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("class SimpleSearch"),
    "1 class matches with regexp==true");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("Simpl.Search") &&
      frame.contentDocument.location.href.includes("regexp=true"),
    "URL is updated");

  TestUtils.clickCheckbox(regExpCheckbox);

  await waitForCondition(
    () => content.textContent.includes("No results for current query"),
    "Nothing matches with regexp==false");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("Simpl.Search") &&
      frame.contentDocument.location.href.includes("regexp=false"),
    "URL is updated");
});

add_task(async function test_SearchPathFilter() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "PathFilter");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Core code (2 lines") &&
      content.textContent.includes("class PathFilter") &&
      content.textContent.includes("Webtest.cpp") &&
      content.textContent.includes("WebtestPathFilter.cpp"),
    "2 classes match without path filter");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("PathFilter") &&
      frame.contentDocument.location.href.includes("path=&"),
    "URL is updated");

  const pathFilter = frame.contentDocument.querySelector("#path");
  TestUtils.setText(pathFilter, "Filter.cpp");

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("class PathFilter") &&
      !content.textContent.includes("Webtest.cpp") &&
      content.textContent.includes("WebtestPathFilter.cpp"),
    "1 class matches without path filter");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("PathFilter") &&
      frame.contentDocument.location.href.includes("path=Filter.cpp&"),
    "URL is updated");

  TestUtils.setText(pathFilter, "");

  await waitForCondition(
    () => content.textContent.includes("Core code (2 lines") &&
      content.textContent.includes("class PathFilter") &&
      content.textContent.includes("Webtest.cpp") &&
      content.textContent.includes("WebtestPathFilter.cpp"),
    "2 classes match without path filter");

  await waitForCondition(
    () => frame.contentDocument.location.href.includes("PathFilter") &&
      frame.contentDocument.location.href.includes("path=&"),
    "URL is updated");
});
