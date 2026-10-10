#include <stdint.h>

namespace mozilla {

// Like modules/libpref/Preferences.h.
class Preferences {
 public:
  static bool GetBool(const char* aPrefName, bool aFallback = false);
  static int32_t GetInt(const char* aPrefName, int32_t aFallback = 0);
  static void RegisterCallback(void (*aCallback)(const char*, void*),
                               const char* aPrefName);
};

}  // namespace mozilla

using mozilla::Preferences;

static void PrefChanged(const char* aPrefName, void* aClosure) {}

int32_t PrefsConsumer() {
  bool enabled = Preferences::GetBool("test.js.pref");
  Preferences::RegisterCallback(PrefChanged, "test.int");
  return mozilla::Preferences::GetInt("test.int") + int32_t(enabled);
}
