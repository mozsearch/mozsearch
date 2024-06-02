#include <stdint.h>

namespace field_layout {

namespace platform_specific_size {

#ifdef TARGET_linux64
using T1 = uint32_t;
#endif

#ifdef TARGET_macosx64
using T1 = uint32_t;
#endif

#ifdef TARGET_win64
using T1 = uint64_t;
#endif

struct S {
  T1 f1;
};

S f() {
  S s;
  return s;
}

}

}
