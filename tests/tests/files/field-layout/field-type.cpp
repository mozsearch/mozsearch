#include <vector>
#include <stddef.h>
#include <stdint.h>

class JSObject {
};

namespace JS {

class TestAllocPolicy {
};


template <typename T>
class Rooted {
};

template <typename T, size_t MinInlineCapacity = 0,
          typename AllocPolicy = TestAllocPolicy>
class GCVector {
};

template <typename T, typename AllocPolicy = TestAllocPolicy>
class StackGCVector : public GCVector<T, 8, AllocPolicy> {
};

template <typename T>
class RootedVector : public Rooted<StackGCVector<T>> {
};

class alignas(8) Value {
 private:
  uint64_t asBits_;
};


template <typename Key, typename Value>
class GCHashMap {
};

}

namespace js {

class TestCheck {
};

template <typename Check, typename T>
class ProtectedDataNoCheckArgs {
};

}

namespace field_layout {

namespace field_type {

struct Type1 {
  uint8_t a;
};

template<typename T>
struct Container1 {
  T a;
};

enum class Enum1 : uint8_t {
  No,
  Yes,
};

template<typename T, Enum1 e>
struct Container2 {
  T a;
};

struct S {
  Type1 value_field;
  const Type1* pointer_field;
  Container1<Type1> template_field_1;
  Container2<Type1, Enum1::No> template_field_2;
  std::vector<Type1> vector_field;
  JS::Rooted<JS::Value> rooted_field;
  JS::RootedVector<JS::Value> rooted_vec_field;
  JS::GCHashMap<JS::Value, JSObject*> hash_map_field;
  js::ProtectedDataNoCheckArgs<js::TestCheck,  JS::Value> protected_field;
};

S f() {
  S s;
  return s;
}

}

}
