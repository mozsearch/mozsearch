#include "TestMetrics.h"

namespace mozilla::glean {

namespace test_metrics {

int probe_one = 0;

int probe_two = 0;

void test() {
  ProbeTwoExtra extra;
  extra.fieldOne = 0;
}

}

}
