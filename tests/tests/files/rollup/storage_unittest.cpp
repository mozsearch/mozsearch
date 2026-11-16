#include "../third_party/googletest/minitest.h"
#include "storage.h"

namespace rollup {

// A test outside test directories (as some gtests are, ex: ipc's
// ports_unittest.cpp), made by a test framework's macro, so test code (see
// third_party/googletest/minitest.h): roll-ups leave out its calls, so
// Rollup Unit Tests has no edges, and those of the framework's code (see the
// check crossref/rollup/rollup_tree__json).
MINI_TEST(StorageUnit) {
  Storage storage;
  storage.Put(4);
  minitest::Check(storage.Get() == 4);
}

}  // namespace rollup
