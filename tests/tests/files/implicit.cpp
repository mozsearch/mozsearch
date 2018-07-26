class Base {
public:
    Base() {
    }

    explicit Base(int foo) {
    }
};

class Derived : public Base {
public:
    Derived() : Base() {}
};

class Implicit : public Base {
public:
    Implicit() {}
};
