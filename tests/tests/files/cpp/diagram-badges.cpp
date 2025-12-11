// for label
class nsAutoRefCnt {
  int value;
};

// for kind
template <typename T>
class nsCOMPtr {
  T* raw;
};

// for elide-and-badge
class nsWrapperCache {
  int cache;
};

namespace diagram_badges {

class C1 {
  int value;
  nsAutoRefCnt mRefCnt;
};

class C2 : nsWrapperCache {
  nsCOMPtr<C1> mPtr;
};

}
