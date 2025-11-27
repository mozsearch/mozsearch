#define NS_ERROR_SEVERITY_SUCCESS 0
#define NS_ERROR_SEVERITY_ERROR 1

#define NS_ERROR_GENERATE(sev, module, code)                            \
  (nsresult)(((uint32_t)(sev) << 31) |                                  \
             ((uint32_t)(module + NS_ERROR_MODULE_BASE_OFFSET) << 16) | \
             ((uint32_t)(code)))

#include "xpcom/base/ErrorList.h"

