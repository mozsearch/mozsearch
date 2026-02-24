#define FUNCTION_START \
  int Function() {

#define FUNCTION_BODY \
  return 1;

#define FUNCTION_END \
  }

namespace range_from_macro {

namespace same_file {

FUNCTION_START
  FUNCTION_BODY
FUNCTION_END

}

}
