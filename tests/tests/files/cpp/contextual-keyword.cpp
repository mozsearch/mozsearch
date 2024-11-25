int final() { return 10; }

int import() { return 20; }

int module() { return 30; }

int override() { return 40; }

int f() { return final() + import() + module() + override(); }

class Base {
 public:
  virtual void Foo();
};

class Sub final : public Base {
 public:
  void Foo() override;
};
