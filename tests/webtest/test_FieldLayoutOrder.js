"use strict";

function getLabels(ns) {
  const labels = [];
  for (const tr of frame.contentDocument.querySelectorAll("#symbol-tree-table-list tbody tr")) {
    let label = tr.querySelector("td").textContent.trim();
    labels.push(label.replace(ns + "::", "").replace(" (base class)", ""));
  }

  return labels.join(",");
}

add_task(async function test_FieldLayoutOrder() {
  for (const ns of ["field_layout::order_no_template",
                     "field_layout::order_template"]) {
    const sym = ns + "::C11";

    await TestUtils.loadQuery("tests", `field-layout:'${sym}'`);

    is(getLabels(ns),
       "C11,,f11,C10,f10,,C9,f9,C8,f8,,C7,,f7,C6,f6,C5,f5,C4,f4,,C3,,f3,C2,f2,C1,f1",
       "Classes are in descending order for " + ns);

    const button = frame.contentDocument.querySelector("#reorder-classes");
    TestUtils.click(button);

    is(getLabels(ns),
       "C1,f1,C2,f2,C3,,f3,C4,f4,,C5,f5,C6,f6,C7,,f7,C8,f8,,C9,f9,C10,f10,,C11,,f11",
       "Classes are in ascending order for " + ns);

    TestUtils.click(button);

    is(getLabels(ns),
       "C11,,f11,C10,f10,,C9,f9,C8,f8,,C7,,f7,C6,f6,C5,f5,C4,f4,,C3,,f3,C2,f2,C1,f1",
       "Classes are in descending order for " + ns);
  }
});
