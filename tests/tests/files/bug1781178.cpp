template <typename> struct Point {
  bool IsThereOne();
};

template <typename T>
struct Foo {
  struct Nested {
    int field;
  };
  Nested nested;

  void Simple();
  static void Static();
  
  template <typename F>
  void Project(Point<F>) {}

  template <typename F>
  void Project(Point<F>, Point<F>) {}

  void Bar() {
    Point<float> p;
    Project(p);

    Point<T> tp;
    this->Project(tp);

    this->Simple();

    (void)nested.field;
  }

  template <typename Other>
  void Baz() {
    Foo<Other>::Static();
  }
};

template <typename T> void TemplateFunc() {
  Point<T> p;
  p.IsThereOne();
}
