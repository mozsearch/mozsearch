"use strict";

add_task(async function test_EmptyQuery() {
  // (The page only has the results JSON when debugging.)
  await TestUtils.loadQuery("tests", "", { debug: "true" });

  const query = frame.contentDocument.querySelector(`#query`);
  ok(query, "query field is shown");
  is(query.value, "", "query field is empty");

  is(JSON.stringify(frame.contentWindow.QUERY_RESULTS_JSON), "[]",
     "Query result for an empty query should be an empty array");
});
