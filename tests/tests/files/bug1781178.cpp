template <typename> struct Point {
  bool IsThereOne();
};

template <typename T>
struct Foo {
  struct Nested {
    int field;
  };
  Nested nested;

  enum E {
    Waldo
  };

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

    (void)E::Waldo;
  }

  template <typename Other>
  void Baz() {
    Foo<Other>::Static();
  }

  using Typedef = int;
};

template <typename T> void TemplateFunc(typename Foo<T>::Typedef) {
  Point<T> p;
  p.IsThereOne();
}
