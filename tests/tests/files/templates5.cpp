#include "templates5.h"

bool func() {
  nsTArray<int> intarray(3);
  nsTArray<float> floatarray(3.5);
  return intarray.Contains(5, 3) || floatarray.Contains(3.5, 5);
}
