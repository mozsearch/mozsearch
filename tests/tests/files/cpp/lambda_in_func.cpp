namespace lambda_in_func {

void func() {
}

void test1() {
  auto lambda = []() {
    func();
  };

  lambda();
}

void caller1() {
  test1();
}

}
