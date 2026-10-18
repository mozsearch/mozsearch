#include "widget.h"

namespace trydelta {

// Shrink a widget twice (which its first commit, which has landed, lets it).
void ShrinkTwice(Widget& aWidget) {
  aWidget.Shrink();
  aWidget.Shrink();
}

}  // namespace trydelta
