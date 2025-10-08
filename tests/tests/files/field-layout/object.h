#include "shape.h"

class JSObject2
    : public js::gc::CellWithTenuredGCPointer<js::gc::Cell, js::Shape> {
  int padding;
  void doJSObject2() {}
};
