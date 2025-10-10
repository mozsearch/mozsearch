#include <stdint.h>

namespace field_layout {

namespace order_no_template {

class C1 {
  int32_t f1;
};

class C2 {
  int8_t f2;
};

class C3 : public C2 {
  int32_t f3;
};

class C4 : public C1, public C3 {
  int8_t f4;
};

class C5 {
  int32_t f5;
};

class C6 {
  int8_t f6;
};

class C7 : public C6 {
  int32_t f7;
};

class C8 : public C5, public C7 {
  int8_t f8;
};

class C9 {
  int32_t f9;
};

class C10 : public C9 {
  int8_t f10;
};

class C11 : public C4, public C8, public C10 {
  int32_t f11;
};

} // order_no_template

namespace order_template {

template <typename T>
class C1 {
  T f1;
};

class C2 {
  int8_t f2;
};

class C3 : public C2 {
  int32_t f3;
};

class C4 : public C1<int32_t>, public C3 {
  int8_t f4;
};

class C5 {
  int32_t f5;
};

class C6 {
  int8_t f6;
};

class C7 : public C6 {
  int32_t f7;
};

class C8 : public C5, public C7 {
  int8_t f8;
};

class C9 {
  int32_t f9;
};

class C10 : public C9 {
  int8_t f10;
};

class C11 : public C4, public C8, public C10 {
  int32_t f11;
};

}  // namespace order_template

}  // namespace field_layout

