#include <stdint.h>

namespace field_layout {

namespace template_multi {

class C1 {
  int32_t f1;
};

class C2 : public C1 {
  int8_t f2;
};

class C3 {
  int32_t f3;
};

template <typename T>
class C4 : public C3 {
  T f4;
};

template <typename B, typename T>
class C5 : public B, public C4<T> {
  int8_t f5;
};

class C6 : C5<C2, int8_t> {
  int32_t f6;
};

C6 f() {
  C6 s;
  return s;
}

}  // namespace template_multi

}  // namespace field_layout
