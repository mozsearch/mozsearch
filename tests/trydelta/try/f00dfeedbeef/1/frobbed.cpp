#include "gadget.h"
#include "widget.h"

namespace trydelta {

// Frobs a widget of a size by an amount, and twice more.
int FrobbedSize(int aSize, int aAmount) {
  Widget widget(aSize);
  widget.Frob(aAmount);
  FrobTwice(widget);
  return widget.Size();
}

// Idles a widget.
void Idle(Widget& aWidget) {
  // Step 1. Set widget's state to "idle".
  aWidget.Frob(-1);
}

}  // namespace trydelta
