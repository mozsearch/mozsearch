#include "../include/BindingTestBinding.h"
#include "./BindingTest.h"

namespace mozilla {
namespace dom {

namespace BindingTest_Binding {

static bool
_constructor() {
  return true;
}

static bool
get_attr1(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTest*>(void_self);
  self->GetAttr1();
  return true;
}

static bool
set_attr1(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTest*>(void_self);
  self->SetAttr1();
  return true;
}

static bool
method1(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTest*>(void_self);
  self->Method1();
  return true;
}

} // BindingTest_Binding

} // namespace dom
} // namespace mozilla

