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
  is(cargoTomlFile.cells.length, 3, "Cargo.toml row has 3 columns");
  is(cargoTomlFile.cells[0].textContent, "Cargo.toml", "Line displays Cargo.toml");
  is(cargoTomlFile.cells[2].textContent, "4327", "Cargo.toml has size");

  const ipdlParserSubmodule = table.rows[5];
  ok(ipdlParserSubmodule.querySelector(".mimetype-fixed-container").classList.contains("mimetype-icon-folder"), "ipdl_parser has folder mime type");
  is(ipdlParserSubmodule.querySelector("a").getAttribute("href"), `${path}/ipdl_parser`, "ipdl_parser has correct link");
  is(ipdlParserSubmodule.cells.length, 3, "ipdl_parser row has 3 columns");
  is(ipdlParserSubmodule.cells[0].textContent, "ipdl_parser", "Line displays ipdl_parser");
  is(ipdlParserSubmodule.cells[2].textContent, "", "ipdl_parser doesn't have size");

  const languagesDirectory = table.rows[6];
  ok(languagesDirectory.querySelector(".mimetype-fixed-container").classList.contains("mimetype-icon-folder"), "languages has folder mime type");
  is(languagesDirectory.querySelector("a").getAttribute("href"), `${path}/languages`, "languages has correct link");
  is(languagesDirectory.cells.length, 3, "languages row has 3 columns");
  is(languagesDirectory.cells[0].textContent, "languages", "Line displays languages");
  is(languagesDirectory.cells[2].textContent, "", "languages doesn't have size");
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
  is(makeTestCommandFile.cells.length, 3, "make_test_command.py row has 3 columns");
  is(makeTestCommandFile.cells[0].textContent, "make_test_command.py", "Line displays make_test_command.py");
  is(makeTestCommandFile.cells[2].textContent, "2345", "make_test_command.py has size");
});
