#include <stdint.h>

namespace mozilla {
namespace StaticPrefs {

inline int32_t
test_int() {
  return 10;
}

inline bool
test_bool() {
  return false;
}

}  // namespace mozilla
}  // namespace StaticPrefs
