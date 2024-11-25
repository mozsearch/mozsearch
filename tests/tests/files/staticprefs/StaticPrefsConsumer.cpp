#include <stdint.h>

#include "bindings/StaticPrefList_test.h"
#include "bindings/StaticPrefList_test2.h"

int32_t StaticPrefsConsumer() {
  int32_t v1 = mozilla::StaticPrefs::test_int();
  uint32_t v2 = mozilla::StaticPrefs::test2_uint();
  bool v3 = mozilla::StaticPrefs::test_bool();

  return int32_t(v1) + int32_t(v2) + int32_t(v3);
}
