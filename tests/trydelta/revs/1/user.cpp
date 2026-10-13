#include "gadget.h"
#include "widget.h"

namespace trydelta {

int UseWidget() {
  Widget widget(3);
  widget.Frob(2);
  FrobTwice(widget);
  return widget.Size();
}

}  // namespace trydelta
