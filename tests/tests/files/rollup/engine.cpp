#include "storage.h"

namespace rollup {

class Engine {
 public:
  void Run(Storage& aStorage) {
    aStorage.Put(1);
    aStorage.Put(aStorage.Get() + 1);
  }
};

}  // namespace rollup
