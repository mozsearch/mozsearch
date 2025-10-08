#include "cell.h"

struct JSClass {
};

namespace js {

class BaseShape : public gc::TenuredCellWithNonGCPointer<const JSClass> {
  void doBaseShape() {}
};

class Shape : public gc::CellWithTenuredGCPointer<gc::TenuredCell, BaseShape> {
  int immutableFlags;
  short objectFlags_;
  void* cache_;
  void doShape() {}
};

}  // namespace js
