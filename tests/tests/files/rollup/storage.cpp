#include "storage.h"

namespace rollup {

void Storage::Put(int aValue) { mValue = aValue; }

int Storage::Get() const { return mValue; }

}  // namespace rollup
