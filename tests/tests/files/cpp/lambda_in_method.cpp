namespace lambda_in_method {

void func() {
}

class C {
 public:
  static void m0() {
    auto lambda = []() {
      func();
    };

    lambda();
  }
};

void caller1() {
  C::m0();
}

}
