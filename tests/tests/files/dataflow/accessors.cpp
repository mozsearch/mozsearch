// See accessors.h.

#include "dataflow/accessors.h"

namespace accessors {

int Measure(Gauge& aGauge, int aInput) {
  aGauge.SetLevel(aInput);
  int level = aGauge.Level();
  return level;
}

int Restart(Gauge& aGauge) {
  aGauge.Reset();
  return aGauge.Level();
}

bool Check(const Gauge& aGauge) { return aGauge.IsReady(); }

}  // namespace accessors
