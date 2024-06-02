#include <stdint.h>

namespace field_layout {

namespace empty {

struct S {
};

S f() {
  S s;
  return s;
}

}

}
