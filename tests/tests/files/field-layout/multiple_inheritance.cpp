#include <stdint.h>

namespace field_layout {

namespace multiple_inheritance {

struct BaseEmpty {};

struct SubA : public BaseEmpty {
  int32_t sub_a_1;
  int16_t sub_a_2;
};

struct SubB : public BaseEmpty {
  int32_t sub_b_1;
  int8_t sub_b_2;
#ifdef TARGET_win64
  int64_t sub_b_3;
#endif
};

struct SubC : public BaseEmpty {
  int8_t sub_c_1;
  int32_t sub_c_2;
};

struct SubD : public SubC {
  int32_t sub_d_1;
  int8_t sub_d_2;
};

struct SubE : public BaseEmpty {
  int64_t sub_e_1;
  int32_t sub_e_2;
};

struct SubSubA : public SubA, public SubB, public SubD {
  int32_t sub_sub_c_1;
};

struct SubSubB : public SubE {
  int32_t sub_sub_b_1;
};

struct SubSubSubA : public SubSubA, public SubSubB {
  int32_t sub_sub_sub_a_1;
};

SubSubSubA f() {
  SubSubSubA s;
  return s;
}

}  // namespace multiple_inheritance

}  // namespace field_layout
