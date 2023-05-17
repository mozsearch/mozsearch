// The purpose of this test case is to illustrate the handling of heuristic
// vs. concrete results in templated code.
//
// Specifically, for the draw() call in foo(), we get all three overloads of
// DrawingContext::draw() as heuristic results, but only draw(Circle) as
// a concrete result (because foo is only instantiated with Shape=Circle).
//
// The current behaviour is to use all the results, so we just get all three
// overloads with nothing to distinguish draw(Circle).
//
// An improved behaviour in the future may be to get all three results,
// but have draw(Circle) annotated differently to indicate that we have
// more confidence in this result than the others.

class GenericSurface {};

class Rectangle {};
class Triangle {};
class Circle {};

template <typename Surface> struct DrawingContext {
  void draw(Rectangle);
  void draw(Triangle);
  void draw(Circle);
};

template <typename Surface, typename Shape>
void foo(DrawingContext<Surface> &d, Shape &s) {
  d.draw(s);
}

int main(void) {
  GenericSurface surface;
  DrawingContext<GenericSurface> context;
  Circle circle;

  foo(context, circle);
}
