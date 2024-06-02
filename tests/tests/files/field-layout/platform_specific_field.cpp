#include <stdint.h>

namespace field_layout {

namespace platform_specific_field {

struct S1 {
  uint32_t f1;

#if defined(TARGET_linux64) || defined(TARGET_macosx64)
  uint32_t f2;
#endif

#ifdef TARGET_win64
  uint8_t f3;
#endif
};

struct S2 : public S1 {
  uint32_t f4;

#ifdef TARGET_linux64
  uint8_t f5;
#endif
};

struct S3 : public S2 {
  uint8_t f6;
};

struct T1 {
  uint8_t f1;
};

struct T2 : public T1 {
  uint32_t f2;

#ifdef TARGET_macosx64
  uint8_t f3;
#else
  uint8_t f3b;
#endif
};

struct T3 : public T2 {
  uint32_t f4;

#if defined(TARGET_linux64) || defined(TARGET_macosx64)
  uint32_t f5;
#endif

#ifdef TARGET_win64
  uint8_t f6;
#endif
};

S3 f() {
  S3 s;
  T3 t;
  return s;
}

}

}
