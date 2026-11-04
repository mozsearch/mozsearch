// A gtest's fake sink (test code, in a gtest directory), which the code
// holding the retrier's lock isn't run with (see locks.cpp).

#include "../sink.h"

namespace mozilla::dom {

class FakeSink : public Sink {
 public:
  void Write() override { mWrites = mWrites + 1; }

 private:
  int mWrites = 0;
};

void UseFakeSink() {
  FakeSink sink;
  sink.Write();
}

}  // namespace mozilla::dom
