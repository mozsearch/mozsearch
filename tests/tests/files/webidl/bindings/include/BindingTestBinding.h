#include <stdint.h>

namespace mozilla {
namespace dom {

namespace BindingTest_Binding {

static const uint32_t CONST_1 = 10;

}  // namespace BindingTest_Binding

struct BindingTestDict {
  unsigned long mProp1;
};

enum class BindingTestEnum {
  Variant1,
  Variant2,
};

}  // namespace dom
}  // namespace mozilla
