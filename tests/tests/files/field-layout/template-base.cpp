#include <stdint.h>

namespace field_layout {

namespace template_base {

// Fully templatized base clas.
template <typename T, typename U>
struct Base {
  T a;
  U b;
};

// Not-templatized subclass that specialized the base class.
struct Sub1 : Base<uint8_t, uint16_t> {
  uint8_t c;
};

// Templatized subclass that partially specializes the base class.
template <typename T, typename V>
struct Sub2 : Base<T, uint16_t> {
  uint32_t c;
  V d;
  uint32_t e;
};

// Not-templatized subclass that specializes the base class (Sub2)
// and its base class (Base).
struct Sub3 : Sub2<char, char16_t> {
  uint32_t f;
};

// Templatized subclass where the base class is template parameter.
template <typename B>
struct Sub4 : B {
  uint32_t x;
};

// Not-templated subclass that specializes the base class (Sub4),
// specifying its base class (Base) with specialization.
struct Sub5 : Sub4<Base<double, float>> {
  uint8_t y;
};

Sub1 f() {
  Sub1 s;
  return s;
}

Sub3 g() {
  Sub3 s;
  return s;
}

Sub5 h() {
  Sub5 s;
  return s;
}

}  // namespace template_base

}  // namespace field_layout
