struct Struct0 {
    void method() {}
};

struct Struct1 {
    void method() const {}
};

void test() {
    const auto lambda = [](auto &&t) {
        t.method();
    };

    lambda(Struct0{});
    lambda(Struct1{});
}
