namespace class_in_method {

void func() {
}

class D1 {
 public:
  static void m0() {
    class C1 {
    public:
      static void m1() {
        func();
      }
    };
    C1::m1();
  }
};

class D2 {
 public:
  static void m0() {
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
};

void caller1() {
  D1::m0();
}

void caller2() {
  D2::m0();
}

}
