namespace class_in_func {

void func() {
}

void test1() {
  class C1 {
  public:
    static void m1() {
      func();
    }
  };
  C1::m1();
}

void test2() {
  class C2 {
  public:
    static void m1() {
      func();
    }
    static void m2() {
      func();
    }
  };
  C2::m1();
  C2::m2();
}

void caller1() {
  test1();
}

void caller2() {
  test2();
}

}
