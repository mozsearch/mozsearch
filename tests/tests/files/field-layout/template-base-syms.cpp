#include <stdint.h>

namespace field_layout {

namespace template_base_syms {

struct Base1 {
  int x;
};

struct Base2 {
  char x;
};

template <typename B>
struct Sub1 : B {
  // This method gets different symbols between specializations.
  void setX() {
    this->x = 10;
  }
};

struct Sub2 : Sub1<Base1> {
};

struct Sub3 : Sub1<Base2> {
};

Sub2 f() {
  Sub2 s;
  return s;
}

Sub3 g() {
  Sub3 s;
  return s;
}

}  // namespace template_base_syms

}  // namespace field_layout
