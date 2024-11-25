#include "../include/BindingTestMixed1Binding.h"
#include "./BindingTestMixed1.h"

namespace mozilla {
namespace dom {

namespace BindingTestMixed1_Binding {

static bool _constructor() { return true; }

static bool ownedMethod1(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed1*>(void_self);
  self->OwnedMethod1();
  return true;
}

static bool get_mixinAttr(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed1*>(void_self);
  self->GetMixinAttr();
  return true;
}

static bool set_mixinAttr(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed1*>(void_self);
  self->SetMixinAttr();
  return true;
}

static bool mixinMethod(void* void_self) {
  auto* self = static_cast<mozilla::dom::BindingTestMixed1*>(void_self);
  self->MixinMethod();
  return true;
}

}  // namespace BindingTestMixed1_Binding

}  // namespace dom
}  // namespace mozilla
