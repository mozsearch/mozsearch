namespace implicit_lambda_use {

void func() {
}

template <typename F>
void callF(F f) {
  f();
}

void test1() {
  callF([]() {
    func();
  });
}

void caller1() {
  test1();
}

};
