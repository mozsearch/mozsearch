"use strict";

add_task(async function test_HistoricalDirectoryView() {
  const path = "/searchfox/rev/a132a39fb2e66eeb13b78ee670dc5372cac05208/tools";
  await TestUtils.loadPath(path);

  const table = frame.contentDocument.querySelector("table.folder-content");
  ok(!!table, "Folder content node exists");
  is(table.rows.length, 1 + 8, "Folder has 8 rows (+ header)");

  const cargoTomlFile = table.rows[3];
  ok(cargoTomlFile.querySelector(".mimetype-fixed-container").classList.contains("mimetype-icon-toml"), "Cargo.toml has toml mime type");
  is(cargoTomlFile.querySelector("a").getAttribute("href"), `${path}/Cargo.toml`, "Cargo.toml has correct link");
  is(cargoTomlFile.cells.length, 4, "Cargo.toml row has 4 columns");
  is(cargoTomlFile.cells[0].textContent, "Cargo.toml", "Line displays Cargo.toml");
  is(cargoTomlFile.cells[2].textContent, "4327", "Cargo.toml has size");

  const ipdlParserSubmodule = table.rows[5];
  ok(ipdlParserSubmodule.querySelector(".mimetype-fixed-container").classList.contains("mimetype-icon-folder"), "ipdl_parser has folder mime type");
  is(ipdlParserSubmodule.querySelector("a").getAttribute("href"), `${path}/ipdl_parser`, "ipdl_parser has correct link");
  is(ipdlParserSubmodule.cells.length, 4, "ipdl_parser row has 4 columns");
  is(ipdlParserSubmodule.cells[0].textContent, "ipdl_parser", "Line displays ipdl_parser");
  is(ipdlParserSubmodule.cells[2].textContent, "", "ipdl_parser doesn't have size");

  const languagesDirectory = table.rows[6];
  ok(languagesDirectory.querySelector(".mimetype-fixed-container").classList.contains("mimetype-icon-folder"), "languages has folder mime type");
  is(languagesDirectory.querySelector("a").getAttribute("href"), `${path}/languages`, "languages has correct link");
  is(languagesDirectory.cells.length, 4, "languages row has 4 columns");
  is(languagesDirectory.cells[0].textContent, "languages", "Line displays languages");
  is(languagesDirectory.cells[2].textContent, "", "languages doesn't have size");

  const revision = frame.contentDocument.querySelector("#revision");
  ok(!!revision, "revision box exists");

  const link = revision.querySelector("a");
  is(link.getAttribute("href"),
     "/searchfox/commit/a132a39fb2e66eeb13b78ee670dc5372cac05208",
     "revision box links to the commit");

  ok(frame.contentDocument.documentElement.classList.contains("old-rev"),
     "the root element has old-rev class");

  const latestLink = frame.contentDocument.querySelector("#panel-vcs-latest");
  ok(!!latestLink, "latst link exists");

  is(latestLink.getAttribute("href"),
     "/searchfox/source/tools",
     "links to the latest view");
});

add_task(async function test_HistoricalDirectoryViewInSubmodule() {
  const path = "/searchfox/rev/a132a39fb2e66eeb13b78ee670dc5372cac05208/tools/ipdl_parser";
  await TestUtils.loadPath(path);

  const table = frame.contentDocument.querySelector("table.folder-content");
  ok(!!table, "Folder content node exists");
  is(table.rows.length, 1 + 10, "Folder has 10 rows (+ header)");

  const makeTestCommandFile = table.rows[8];
  ok(makeTestCommandFile.querySelector(".mimetype-fixed-container").classList.contains("mimetype-icon-py"), "make_test_command.py has python mime type");
  is(makeTestCommandFile.querySelector("a").getAttribute("href"), `${path}/make_test_command.py`, "make_test_command.py has correct link");
  is(makeTestCommandFile.cells.length, 4, "make_test_command.py row has 4 columns");
  is(makeTestCommandFile.cells[0].textContent, "make_test_command.py", "Line displays make_test_command.py");
  is(makeTestCommandFile.cells[2].textContent, "2345", "make_test_command.py has size");

  const revision = frame.contentDocument.querySelector("#revision");
  ok(!!revision, "revision box exists");

  const link = revision.querySelector("a");
  is(link.getAttribute("href"),
     "/searchfox/commit/a132a39fb2e66eeb13b78ee670dc5372cac05208",
     "revision box links to the commit");

  ok(frame.contentDocument.documentElement.classList.contains("old-rev"),
     "the root element has old-rev class");

  const latestLink = frame.contentDocument.querySelector("#panel-vcs-latest");
  ok(!!latestLink, "latst link exists");

  is(latestLink.getAttribute("href"),
     "/searchfox/source/tools/ipdl_parser",
     "links to the latest view");
});

add_task(async function test_HistoricalViewNotFound() {
  // test_HistoricalViews.js is added by 8c206d60bf8ff33097065f5da02172f56864fac8,
  // and a132a39fb2e66eeb13b78ee670dc5372cac05208 is the parent of the commit.
  //
  // If an user clicks "Show latest version without this line" for the blame popup for
  // the 8c206d revision, the page for the a132a3 revision is opened.
  //
  // The page should explicitly say the file does not exist.
  {
    const path = "/searchfox/rev/8c206d60bf8ff33097065f5da02172f56864fac8/tests/webtest/test_HistoricalViews.js";
    await TestUtils.loadPath(path);

    const file = frame.contentDocument.querySelector("#file");
    ok(!!file, "File is shown if the revision has the file.");
  }

  {
    const path = "/searchfox/rev/a132a39fb2e66eeb13b78ee670dc5372cac05208/tests/webtest/test_HistoricalViews.js";
    await TestUtils.loadPath(path);

    const file = frame.contentDocument.querySelector("#file");
    ok(!file, "File is not shown if the revision does not have the file.");

    const table = frame.contentDocument.querySelector("table.folder-content");
    ok(!table, "Folder is not shown if the revision does not have the file");

    is(frame.contentDocument.body.textContent,
       "File, directory, or parent git submodule not found",
       "Error message is shown");
  }
});

add_task(async function test_BlamePopupInDiffView() {
  // test_HistoricalViewNotFound above is added by 7ebfd0db68e3105d0b869676af7fc4ce8b08ea1b
  // while the previous test functions were added by 8c206d60bf8ff33097065f5da02172f56864fac8.
  //
  // The blame popup for the diff of 7ebfd0db should point at 8c206d60 for line 47 and 7ebfd0db
  // itself for line 48.
  const path = "/searchfox/diff/7ebfd0db68e3105d0b869676af7fc4ce8b08ea1b/tests/webtest/test_HistoricalViews.js";
  await TestUtils.loadPath(path);

  const blamePopup = frame.contentDocument.querySelector(`#blame-popup`);

  const line47 = frame.contentDocument.querySelector("#line-47");
  line47.querySelector(".blame-strip").click();
  await waitForShown(blamePopup);
  ok(blamePopup.querySelector(`.blame-entry`).textContent.includes("Bug 1516970"));

  line47.querySelector(".blame-strip").click();
  await waitForHidden(blamePopup);

  const line48 = frame.contentDocument.querySelector("#line-48");
  line48.querySelector(".blame-strip").click();
  await waitForShown(blamePopup);
  ok(blamePopup.querySelector(`.blame-entry`).textContent.includes("Bug 2008286"));
});
