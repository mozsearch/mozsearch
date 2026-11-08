// A gadget's pref, whose use the semantic analysis adds to the compilation's
// analysis of the file.
namespace mozilla {
struct Preferences {
  static bool GetBool(const char* aPref) { return aPref[0] != '\0'; }
};
}  // namespace mozilla

namespace trydelta {

bool GadgetIsLoud() { return mozilla::Preferences::GetBool("gadget.loudness"); }

}  // namespace trydelta
