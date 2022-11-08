template <typename> struct Point {};

template <typename>
struct Foo {
  template <typename F>
  void Project(Point<F>);

  void Bar() {
    Point<float> p;
    Project(p);
  }
};
