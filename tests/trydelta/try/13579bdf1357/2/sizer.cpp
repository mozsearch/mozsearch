#include "widget.h"

namespace trydelta {

// Sizes widgets.
class Sizer {
 public:
  int SizeOf(Widget& aWidget) { return aWidget.Size(); }
};

int UseSizer() {
  Widget widget(3);
  Sizer sizer;
  return sizer.SizeOf(widget);
}

}  // namespace trydelta
