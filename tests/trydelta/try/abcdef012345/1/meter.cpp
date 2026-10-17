#include "widget.h"

namespace trydelta {

// Meters a widget, shrinking it as it goes.
class Meter {
 public:
  int Measure(Widget& aWidget) {
    aWidget.Shrink();
    return aWidget.Size();
  }
};

int UseMeter() {
  Widget widget(4);
  Meter meter;
  return meter.Measure(widget);
}

}  // namespace trydelta
