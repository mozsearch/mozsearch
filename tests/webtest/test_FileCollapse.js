"use strict";

add_task(async function test_FileCollapse_Interactive() {
  await TestUtils.loadPath("/tests/search?q=SimpleSearch&redirect=false");
  TestUtils.shortenSearchTimeouts();

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("class SimpleSearch"),
    "Search results loaded"
  );

  const fileExpando = content.querySelector(".result-head .expando");
  ok(fileExpando, "File expando arrow exists");

  const style = frame.contentWindow.getComputedStyle(fileExpando);
  is(style.visibility, "visible", "Expando arrow should be visible");

  const targetClass = fileExpando.getAttribute("data-klass");
  const rows = content.querySelectorAll("." + targetClass);
  isnot(rows.length, 0, "There should be result rows associated with this file");

  is(rows[0].style.display, "", "Rows should be visible initially");

  TestUtils.click(fileExpando);
  is(rows[0].style.display, "none", "Rows should be hidden after click");

  TestUtils.click(fileExpando);
  is(rows[0].style.display, "", "Rows should be visible after second click");
});

add_task(async function test_FileCollapse_Interaction() {
  await TestUtils.loadPath("/tests/search?q=SimpleSearch&redirect=false");
  TestUtils.shortenSearchTimeouts();

  const content = frame.contentDocument.querySelector("#content");
  await waitForCondition(
    () => content.textContent.includes("Definitions"),
    "Definitions section loaded"
  );

  const sectionExpando = content.querySelector(".expando[data-klass^='EXPANDO']");
  const fileExpando = content.querySelector(".result-head .expando");
  const targetClass = fileExpando.getAttribute("data-klass");
  const rows = content.querySelectorAll("." + targetClass);

  TestUtils.click(sectionExpando);
  is(fileExpando.offsetParent, null, "File header should be hidden (collapsed by section)");

  TestUtils.click(sectionExpando);
  isnot(fileExpando.offsetParent, null, "File header should be visible again");
  is(rows[0].style.display, "", "File rows should be visible");

  TestUtils.click(fileExpando);
  is(rows[0].style.display, "none", "File rows hidden");

  TestUtils.click(sectionExpando);
  is(fileExpando.offsetParent, null, "File header hidden by section");

  TestUtils.click(sectionExpando);
  isnot(fileExpando.offsetParent, null, "File header is visible");
  is(rows[0].style.display, "none", "File rows MUST remain hidden because file was closed");
});

add_task(async function test_FileCollapse_HiddenForFiles() {
  await TestUtils.loadPath("/tests/search?q=Webtest.cpp&redirect=false");
  TestUtils.shortenSearchTimeouts();

  const content = frame.contentDocument.querySelector("#content");
  await waitForCondition(
    () => content.textContent.includes("Files") && content.textContent.includes("Webtest.cpp"),
    "Filename search results loaded"
  );

  const fileExpando = content.querySelector(".result-head .expando");
  const style = frame.contentWindow.getComputedStyle(fileExpando);

  is(style.visibility, "hidden", "Expando arrow should be hidden for file-only matches");
});
