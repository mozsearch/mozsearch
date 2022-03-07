#include "dist/include/xpctest_params.h"

void consume_xpidl(nsIXPCTestParams *params) {
  uint8_t b = 0;
  uint8_t out;
  params->TestOctet(1, &b, &out);
}
