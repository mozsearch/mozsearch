interface BindingTest {
  constructor();

  const unsigned long CONST_1 = 10;

  attribute any attr1;

  any method1();
};

dictionary BindingTestDict {
  unsigned long prop1;
};

enum BindingTestEnum {
  "variant1",
  "variant2",
};
