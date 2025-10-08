#include "shape.h"

template
class js::gc::CellWithTenuredGCPointer<js::gc::Cell, js::Shape>;

namespace field_layout {

namespace template_base_shape {

js::Shape f() {
  js::Shape s;
  return s;
}

}  // namespace template_base_shape

}  // namespace field_layout
