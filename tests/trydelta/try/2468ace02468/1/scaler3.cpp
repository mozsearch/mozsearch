#define SCALER_VERSION 3
#include "scaler.h"
#include "scaler_impl.inc"

namespace trydelta {
int ScaleBy3(int aValue) { return Scaler<3>::Scale(aValue); }
}  // namespace trydelta
