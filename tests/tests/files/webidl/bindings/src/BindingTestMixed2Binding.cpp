#include "../include/BindingTestMixed2Binding.h"
#include "./BindingTestMixed2.h"

namespace mozilla {
namespace dom {

namespace BindingTestMixed2_Binding {

static bool
_constructor() {
  return true;
}

static bool
ownedMethod2(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed2*>(void_self);
  self->OwnedMethod2();
  return true;
}

static bool
get_mixinAttr(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed2*>(void_self);
  self->GetMixinAttr();
  return true;
}

static bool
set_mixinAttr(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed2*>(void_self);
  self->SetMixinAttr();
  return true;
}

static bool
mixinMethod(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed2*>(void_self);
  self->MixinMethod();
  return true;
}

} // BindingTestMixed2_Binding

} // namespace dom
} // namespace mozilla

