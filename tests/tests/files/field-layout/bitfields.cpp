#include <stdint.h>

namespace field_layout {

namespace bitfields {

struct S {
  uint32_t b1: 1;
  uint32_t b2: 3;
  uint32_t b3: 7;
  uint32_t b4: 4;
  uint8_t  b5: 3;
  uint16_t b6: 2;
};

S f() {
  S s;
  return s;
}

}

}
