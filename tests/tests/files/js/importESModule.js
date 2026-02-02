const { func1 } = ChromeUtils.importESModule("resource:///some/foo1.mjs");
const { func2: func2b } = ChromeUtils.importESModule("resource:///some/foo2.mjs");
const func3 = ChromeUtils.importESModule("resource:///some/foo3.mjs").func3;
const func4b = ChromeUtils.importESModule("resource:///some/foo4.mjs").func4;

const { func5 } = SpecialPowers.ChromeUtils.importESModule("resource:///some/foo5.mjs");
const { func6: func6b } = SpecialPowers.ChromeUtils.importESModule("resource:///some/foo6.mjs");
const func7 = SpecialPowers.ChromeUtils.importESModule("resource:///some/foo7.mjs").func7;
const func8b = SpecialPowers.ChromeUtils.importESModule("resource:///some/foo8.mjs").func8;

[
  func1,
  func2b,
  func3,
  func4b,
  func5,
  func6b,
  func7,
  func8b,
];
