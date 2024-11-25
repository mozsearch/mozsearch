#include "dist/include/xpctest_attributes.h"
#include "dist/include/xpctest_params.h"

void consume_xpidl(nsIXPCTestParams* params) {
  uint8_t b = 0;
  uint8_t out;
  params->TestOctet(1, &b, &out);
}

void consume_attr(nsIXPCTestObjectReadWrite* attrs) {
  bool string_was_too_hard;

  attrs->GetBooleanProperty(&string_was_too_hard);

  // Yup, now put it back!
  attrs->SetBooleanProperty(string_was_too_hard);
}
