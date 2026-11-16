#pragma once

// A test framework (a miniature googletest; see diagram-mapping.toml's
// `[tests]` frameworks), whose code is test code wherever it's used: a test
// made by its macro is defined where it's used, and, as the macro spells its
// method, here, so it's test code wherever its file is (see crossref's
// `rollup_left_out`, and rollup/storage_unittest.cpp).

namespace minitest {

inline bool Check(bool aCondition) { return aCondition; }

}  // namespace minitest

#define MINI_TEST(Name)  \
  class Name##_Test {    \
   public:               \
    void TestBody();     \
  };                     \
  void Name##_Test::TestBody()
