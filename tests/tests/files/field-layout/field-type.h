class JSObject {
};

namespace JS {

class TestAllocPolicy {
};


template <typename T>
class Rooted {
};

template <typename T, size_t MinInlineCapacity = 0,
          typename AllocPolicy = TestAllocPolicy>
class GCVector {
};

template <typename T, typename AllocPolicy = TestAllocPolicy>
class StackGCVector : public GCVector<T, 8, AllocPolicy> {
};

template <typename T>
class RootedVector : public Rooted<StackGCVector<T>> {
};

class alignas(8) Value {
 private:
  uint64_t asBits_;
};


template <typename Key, typename Value>
class GCHashMap {
};

}

namespace js {

class TestCheck {
};

template <typename Check, typename T>
class ProtectedDataNoCheckArgs {
};

template <typename T>
using TestLockData = ProtectedDataNoCheckArgs<TestCheck, T>;

}
