#pragma once

namespace trydelta {

// A trunk revision's, after the indexed one: not the push's.
inline int CountWidgets() {
  return 2;
}

// The push's, which needs the trunk's counter.h.
inline int CountGadgets() { return CountWidgets() + 1; }

}  // namespace trydelta
