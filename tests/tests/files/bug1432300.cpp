/** This simulates the code from JS::ProfilingFrameIterator::getPhysicalFrameAndEntry
  * which was highlighting incorrectly in bug 1432300 */

namespace mozilla {

template<class T> class Maybe {
public:
  Maybe(T x) : mX(x) {
  }

  Maybe(const Maybe<T>& x) : mX(x.mX) {
  }

  T mX;
};

}

mozilla::Maybe<int> getAThing() {
  return mozilla::Maybe<int>(42);
}

void useAThing() {
  mozilla::Maybe<int> thing = getAThing();
}
