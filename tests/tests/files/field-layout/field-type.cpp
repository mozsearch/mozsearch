#include <vector>
#include <stddef.h>
#include <stdint.h>
#include "field-type.h"

namespace field_layout {

namespace field_type {

struct Type1 {
  uint8_t a;
};

template <typename T>
struct Container1 {
  T a;
};

enum class Enum1 : uint8_t {
  No,
  Yes,
};

template <typename T, Enum1 e>
struct Container2 {
  T a;
};

#define DEFINE_FIELDS   \
  Type1 macro_fields_1; \
  Enum1 macro_fields_2; \
  Container1<Type1> macro_fields_3;

#define SOME_ANNOTATION __attribute__((annotate("some_annotation")))

struct S {
  Type1 value_field;
  const Type1* pointer_field;
  Container1<Type1> template_field_1;
  Container2<Type1, Enum1::No> template_field_2;
  std::vector<Type1> vector_field;
  JS::Rooted<JS::Value> rooted_field;
  JS::RootedVector<JS::Value> rooted_vec_field;
  JS::GCHashMap<JS::Value, JSObject*> hash_map_field;
  js::TestLockData<JS::Value> protected_field;
  DEFINE_FIELDS
  js::TestLockData<Type1> multiline_field SOME_ANNOTATION;
#include "field-type-include.h"
};

S f() {
  S s;
  return s;
}

}  // namespace field_type

}  // namespace field_layout
