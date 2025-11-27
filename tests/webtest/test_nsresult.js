"use strict";

add_task(async function test_nsresult() {
  const tests = [
    {
      query: "nsresult:0",
      desc: "nsresult(0x0) is NS_OK / NS_BINDING_SUCCEEDED",
      defs: [
        "NS_OK = 0x0",
        "NS_BINDING_SUCCEEDED = 0x0",
        "NS_OK = nsresult::NS_OK",
        "NS_BINDING_SUCCEEDED = nsresult::NS_BINDING_SUCCEEDED",
      ],
    },
    {
      query: "nsresult:0x0",
      desc: "nsresult(0x0) is NS_OK / NS_BINDING_SUCCEEDED",
      defs: ["NS_OK = 0x0"],
    },
    {
      query: "nsresult:0x8000FFFF",
      desc: "nsresult(0x8000ffff) is NS_ERROR_UNEXPECTED",
      defs: [
        "NS_ERROR_UNEXPECTED = 0x8000FFFF",
        "NS_ERROR_UNEXPECTED = nsresult::NS_ERROR_UNEXPECTED",
      ],
    },
    {
      query: "nsresult:8000FFFF",
      desc: "nsresult(0x8000ffff) is NS_ERROR_UNEXPECTED",
      defs: ["NS_ERROR_UNEXPECTED = 0x8000FFFF"],
    },
    {
      query: "nsresult:2147549183",
      desc: "nsresult(0x8000ffff) is NS_ERROR_UNEXPECTED",
      defs: ["NS_ERROR_UNEXPECTED = 0x8000FFFF"],
    },
    {
      query: "nsresult:0x80460012",
      desc: "nsresult(0x80460012) is NS_ERROR_GENERATE(NS_ERROR_SEVERITY_ERROR, NS_ERROR_MODULE_XPCOM, 0x12)",
      defs: [
        "#define NS_ERROR_MODULE_XPCOM",
        "#define NS_ERROR_SEVERITY_ERROR",
        "#define NS_ERROR_GENERATE",
      ],
    },
    {
      query: "nsresult:0x00460012",
      desc: "nsresult(0x460012) is NS_ERROR_GENERATE(NS_ERROR_SEVERITY_SUCCESS, NS_ERROR_MODULE_XPCOM, 0x12)",
      defs: [
        "#define NS_ERROR_MODULE_XPCOM",
        "#define NS_ERROR_SEVERITY_SUCCESS",
        "#define NS_ERROR_GENERATE",
      ],
    },
    {
      query: "nsresult:0x80470012",
      desc: "nsresult(0x80470012) is NS_ERROR_GENERATE(NS_ERROR_SEVERITY_ERROR, 0x2, 0x12)",
      defs: [
        "#define NS_ERROR_SEVERITY_ERROR",
        "#define NS_ERROR_GENERATE",
      ],
    },
    {
      query: "nsresult:0x80440012",
      desc: "nsresult(0x80440012) is unknown",
      defs: null,
    },
    {
      query: ["nsresult:01g"],
      desc: "nsresult(01g) is unknown",
      defs: null,
    },
  ];

  for (const test of tests) {
    await TestUtils.loadPath("/");
    TestUtils.shortenSearchTimeouts();

    const query = frame.contentDocument.querySelector("#query");
    TestUtils.setText(query, test.query);

    await waitForCondition(
      () => frame.contentDocument.querySelector(".nsresult-desc"),
      "nsresult description is shown for " + test.query);

    const desc = frame.contentDocument.querySelector(".nsresult-desc");
    is(desc.textContent, test.desc,
       "description matches for " + test.query);

    const content = frame.contentDocument.querySelector("#content");

    if (test.defs) {
      await waitForCondition(
        () => test.defs.every(d => content.textContent.includes(d)),
        "definitions are shown for " + test.query);
    }
  }
});
