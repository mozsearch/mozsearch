class Base {
public:
  virtual void Foo();
};

class Sub final : public Base {
public:
  void Foo() override;
};

int f() {
  int final = 10;
  int import = 20;
  int module = 30;
  int override = 40;

  return final + import + module + override;
}
