template <typename> struct Point {
  bool IsThereOne();
};

template <typename>
struct Foo {
  void Simple();
  static void Static();
  
  template <typename F>
  void Project(Point<F>) {}

  template <typename F>
  void Project(Point<F>, Point<F>) {}

  void Bar() {
    Point<float> p;
    Project(p);

    this->Simple();
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
