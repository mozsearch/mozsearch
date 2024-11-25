#include <stdint.h>

namespace field_layout {

namespace holes {

struct Base {
  uint8_t a;
  uint16_t b;
  uint32_t c;
  char d;
};

struct Sub : public Base {
  uint8_t x;
  uint32_t y;
};

Sub f(int32_t n) {
  Sub s;
  s.y = n;

  return s;
}

}  // namespace holes

}  // namespace field_layout
