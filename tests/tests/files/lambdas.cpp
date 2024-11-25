struct Struct0 {
  void method() {}
};

struct Struct1 {
  void method() const {}
};

void test() {
  const auto lambda = [](auto&& t) { t.method(); };

  lambda(Struct0{});
  lambda(Struct1{});

  const auto capture_all_by_reference = [&] { (void)lambda; };
  const auto capture_all_by_value = [=] { (void)lambda; };
  const auto capture_one_by_reference = [&lambda] { (void)lambda; };
  const auto capture_one_by_value = [lambda] { (void)lambda; };
  const auto capture_by_named_reference = [&lambda = lambda] { (void)lambda; };
  const auto capture_by_named_value = [lambda = lambda] { (void)lambda; };
}
