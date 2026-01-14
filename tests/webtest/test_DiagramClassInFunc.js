"use strict";

add_task(async function test_DiagramClassInFunc() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'class_in_func::func' depth:4");

  // All functions/classes should be rendered as separate nodes.
  const nodeSyms = [
    "_ZN13class_in_func7caller1Ev",
    "_ZN13class_in_func5test1Ev",
    "T_class_in_func::test1::C1",
    "_ZZN13class_in_func5test1EvEN2C12m1Ev",

    "_ZN13class_in_func7caller2Ev",
    "_ZN13class_in_func5test2Ev",
    "T_class_in_func::test2::C2",
    "_ZZN13class_in_func5test2EvEN2C22m1Ev",
    "_ZZN13class_in_func5test2EvEN2C22m2Ev",

    "_ZN13class_in_func4funcEv",
  ];

  for (const sym of nodeSyms) {
    const node = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(node, `${sym} node exists`);
  }

  const edgeSyms = [
    "_ZN13class_in_func7caller1Ev->_ZN13class_in_func5test1Ev",
    "_ZN13class_in_func5test1Ev->T_class_in_func::test1::C1:_ZZN13class_in_func5test1EvEN2C12m1Ev",
    "T_class_in_func::test1::C1:_ZZN13class_in_func5test1EvEN2C12m1Ev->_ZN13class_in_func4funcEv",

    "_ZN13class_in_func7caller2Ev->_ZN13class_in_func5test2Ev",
    "_ZN13class_in_func5test2Ev->T_class_in_func::test2::C2:_ZZN13class_in_func5test2EvEN2C22m1Ev",
    "_ZN13class_in_func5test2Ev->T_class_in_func::test2::C2:_ZZN13class_in_func5test2EvEN2C22m2Ev",
    "T_class_in_func::test2::C2:_ZZN13class_in_func5test2EvEN2C22m1Ev->_ZN13class_in_func4funcEv",
    "T_class_in_func::test2::C2:_ZZN13class_in_func5test2EvEN2C22m2Ev->_ZN13class_in_func4funcEv",
  ];

  for (const sym of edgeSyms) {
    const edge = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(edge, `${sym} edge exists`);
  }
});

add_task(async function test_DiagramClassInMethod() {
  await TestUtils.resetFeatureGate("diagramming");

  await TestUtils.loadQuery("tests", "calls-to:'class_in_method::func' depth:4");

  // All functions/classes should be rendered as separate nodes.
  const nodeSyms = [
    "_ZN15class_in_method7caller1Ev",
    "T_class_in_method::D1",
    "T_class_in_method::D1::m0::C1",
    "_ZN15class_in_method2D12m0Ev",
    "_ZZN15class_in_method2D12m0EvEN2C12m1Ev",

    "_ZN15class_in_method7caller2Ev",
    "T_class_in_method::D2",
    "T_class_in_method::D2::m0::C2",
    "_ZN15class_in_method2D22m0Ev",
    "_ZZN15class_in_method2D22m0EvEN2C22m1Ev",
    "_ZZN15class_in_method2D22m0EvEN2C22m2Ev",

    "_ZN15class_in_method4funcEv",
  ];

  for (const sym of nodeSyms) {
    const node = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(node, `${sym} node exists`);
  }

  const edgeSyms = [
    "_ZN15class_in_method7caller1Ev->T_class_in_method::D1:_ZN15class_in_method2D12m0Ev",
    "T_class_in_method::D1:_ZN15class_in_method2D12m0Ev->T_class_in_method::D1::m0::C1:_ZZN15class_in_method2D12m0EvEN2C12m1Ev",
    "T_class_in_method::D1::m0::C1:_ZZN15class_in_method2D12m0EvEN2C12m1Ev->_ZN15class_in_method4funcEv",

    "_ZN15class_in_method7caller2Ev->T_class_in_method::D2:_ZN15class_in_method2D22m0Ev",
    "T_class_in_method::D2:_ZN15class_in_method2D22m0Ev->T_class_in_method::D2::m0::C2:_ZZN15class_in_method2D22m0EvEN2C22m1Ev",
    "T_class_in_method::D2:_ZN15class_in_method2D22m0Ev->T_class_in_method::D2::m0::C2:_ZZN15class_in_method2D22m0EvEN2C22m2Ev",
    "T_class_in_method::D2::m0::C2:_ZZN15class_in_method2D22m0EvEN2C22m1Ev->_ZN15class_in_method4funcEv",
    "T_class_in_method::D2::m0::C2:_ZZN15class_in_method2D22m0EvEN2C22m2Ev->_ZN15class_in_method4funcEv",
  ];

  for (const sym of edgeSyms) {
    const edge = frame.contentDocument.querySelector(`[data-symbols="${sym}"]`);
    ok(edge, `${sym} edge exists`);
  }
});
