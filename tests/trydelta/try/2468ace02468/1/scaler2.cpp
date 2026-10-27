#define SCALER_VERSION 2
#include "scaler.h"
#include "scaler_impl.inc"

namespace trydelta {
int ScaleBy2(int aValue) { return Scaler<2>::Scale(aValue); }
}  // namespace trydelta
