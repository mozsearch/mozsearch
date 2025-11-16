#include <stdint.h>

namespace errors_msg_file {

enum JSErrNum {
#define MSG_DEF(name, count, exception, format) name,
#include "errors.msg"
#undef MSG_DEF
  JSErr_Limit
};

enum JSExnType {
  JSEXN_ERR,
  JSEXN_REFERENCEERR,
};

struct JSErrorFormatString {
  const char* name;
  const char* format;
  uint16_t argCount;
  int16_t exnType;
};

const JSErrorFormatString js_ErrorFormatString[JSErr_Limit] = {
#define MSG_DEF(name, count, exception, format) \
  {#name, format, count, exception},
#include "errors.msg"
#undef MSG_DEF
};

}
