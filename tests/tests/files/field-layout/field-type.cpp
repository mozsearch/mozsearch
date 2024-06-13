#include <vector>
#include <stdint.h>

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
};

S f() {
  S s;
  return s;
}

}

}
