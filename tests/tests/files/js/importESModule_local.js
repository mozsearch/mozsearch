(function () {
  const { func1 } = ChromeUtils.importESModule("resource:///some/foo1.mjs");
  const { func2: func2b } = ChromeUtils.importESModule("resource:///some/foo2.mjs");
  const func3 = ChromeUtils.importESModule("resource:///some/foo3.mjs").func3;
  const func4b = ChromeUtils.importESModule("resource:///some/foo4.mjs").func4;

  return [
    func1,
    func2b,
    func3,
    func4b,
  ];
})();
