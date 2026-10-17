// See model.h.

#include "rollup/model.h"

namespace rollup {

static int Normalize(int aValue) { return aValue < 0 ? 0 : aValue; }

// (Not just a setter: its write of its own `mStats` isn't data moving to
// another class.)
void Store::Put(int aValue) {
  mValue = Normalize(aValue);
  mStats.mLast = aValue;
}

}  // namespace rollup
