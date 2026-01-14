#include <stdint.h>

namespace diagram_merge {

#ifdef TARGET_linux64
using T1 = uint32_t;
#endif

#ifdef TARGET_macosx64
using T1 = uint32_t;
#endif

#ifdef TARGET_win64
using T1 = uint64_t;
#endif

void foo(T1 n) {
}

}
