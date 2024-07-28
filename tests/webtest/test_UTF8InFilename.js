add_task(async function test_UTF8InFilename_Search() {
  await TestUtils.loadPath("/");
  TestUtils.shortenSearchTimeouts();

  const query = frame.contentDocument.querySelector("#query");
  TestUtils.setText(query, "SymbolInFilenameWithUTF8");

  const content = frame.contentDocument.querySelector("#content");

  await waitForCondition(
    () => content.textContent.includes("Core code (1 lines") &&
      content.textContent.includes("Definitions (SymbolInFilenameWithUTF8) (1 lines") &&
      content.textContent.includes("var SymbolInFilenameWithUTF8"),
    "symbol in file with space in filename matches as definition");
});

add_task(async function test_UTF8InFilename_DirAndFile() {
  await TestUtils.loadPath("/tests/source/js");

  const link = frame.contentDocument.querySelector(`.folder-content a[href*="with-UTF8-"]`);
  ok(!!link, "UTF-8 filename is listed");
  is(link.getAttribute("href"), "/tests/source/js/with-UTF8-ファイル.js",
     "UTF-8 path is used with raw text");

  const loadPromise = TestUtils.waitForLoad();
  TestUtils.click(link);
  await loadPromise;

  ok(frame.contentDocument.location.href.includes(
       "with-UTF8-%E3%83%95%E3%82%A1%E3%82%A4%E3%83%AB.js"),
     "Navigated to the file page with URL-encoded path");

  const breadcrumbs = frame.contentDocument.querySelector(`.breadcrumbs`);
  ok(breadcrumbs.textContent.includes("/js/with-UTF8-ファイル.js"),
     "UTF-8 path is written in breadcrumbs with raw text");
});
