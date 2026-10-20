#include "../storage.h"

namespace rollup {

// A test of Storage (in a test file, by its path kind), whose calls roll-ups
// leave out (see crossref's `test_code`): Rollup Tests has no edge to Rollup
// Storage.
class StorageTest {
 public:
  int Run(Storage& aStorage) {
    aStorage.Put(3);
    return aStorage.Get();
  }
};

}  // namespace rollup
