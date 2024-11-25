#include <stdint.h>

namespace field_layout {

namespace empty_subclass {

struct S {
  uint32_t x;
};

struct T : public S {};

T f() {
  T t;
  return t;
}

}  // namespace empty_subclass

}  // namespace field_layout
