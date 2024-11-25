#include <stdint.h>

namespace field_layout {

namespace vtable {

class Base1 {
 public:
  virtual ~Base1() {}
};

class Base2 {
 public:
  virtual ~Base2() {}
};

class Base3 {
 public:
};

class Sub1a : public Base1 {
 public:
  virtual ~Sub1a() {}
  uint8_t x;
};

class Sub1b : public Base1 {
 public:
  virtual ~Sub1b() {}
  uint8_t y;
};

class Sub2 : public Base2 {
 public:
  virtual ~Sub2() {}
  uint8_t w;
};

class Sub3 : public Base3 {
 public:
  virtual ~Sub3() {}
  uint8_t v;
};

class SubSub : public Sub1a, public Sub1b, public Sub2, public Sub3 {
 public:
  virtual ~SubSub() {}
  uint8_t z;
};

SubSub f(int8_t n) {
  SubSub s;
  s.x = n;

  return s;
}

}  // namespace vtable

}  // namespace field_layout
