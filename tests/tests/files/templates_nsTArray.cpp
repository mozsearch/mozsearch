#include "nsTArray.h"

struct ServoAttrSnapshot {};

class ServoElementSnapshot {
  nsTArray<ServoAttrSnapshot> mAttrs;
};
