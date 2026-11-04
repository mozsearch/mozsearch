// A sink a retrier writes to with its lock held (see locks.cpp), which a
// file's sink implements, and a gtest's fake (gtest/FakeSink.cpp).

#ifndef locks_sink_h
#define locks_sink_h

namespace mozilla::dom {

class Sink {
 public:
  virtual void Write() = 0;
};

}  // namespace mozilla::dom

#endif
