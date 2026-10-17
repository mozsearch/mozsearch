// See params.h.

#include "params/params.h"

namespace params {

int Scale(int aValue, int aFactor) { return aValue * aFactor; }

int Second(int, bool aSecond) { return aSecond ? 1 : 0; }

int Third(int, int aThird) { return aThird; }

int UseThem() { return Scale(Second(1, true), Third(2, 3)); }

}  // namespace params
