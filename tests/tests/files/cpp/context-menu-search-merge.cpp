#include <stdint.h>

namespace search_merge {

namespace ns1 {
  class C {
   public:
    C() {}
  };
}

namespace ns2 {
  class C {
   public:
    C() {}
  };
}

namespace ns3 {
  class C {
   public:
    C() {}
  };
}

namespace ns4 {
  class C {
   public:
    C() {}
  };
}

#ifdef DEBUG

#ifdef TARGET_linux64
#define ns ns1
#endif

#ifdef TARGET_macosx64
#define ns ns2
#endif

#ifdef TARGET_win64
#define ns ns3
#endif

void func(const ns::C& p) {
}

#else  // opt

#define ns ns4

void func(const ns::C& p) {
}

#endif

void caller() {
  ns::C x;
  func(x);
}

}  // search_merge
