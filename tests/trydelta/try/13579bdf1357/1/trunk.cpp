#include "widget.h"

namespace trydelta {

// A trunk revision's, after the indexed one: not the push's.
int TrunkOnly() {
  Widget widget(2);
  return widget.Size();
}

}  // namespace trydelta
