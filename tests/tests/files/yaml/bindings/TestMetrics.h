#ifndef test_TestMetrics_h
#define test_TestMetrics_h

namespace mozilla::glean {

namespace test_metrics {
  extern int probe_one;

  struct ProbeTwoExtra {
    int fieldOne;
  };

  extern int probe_two;
}

}
#endif // test_TestMetrics_h
