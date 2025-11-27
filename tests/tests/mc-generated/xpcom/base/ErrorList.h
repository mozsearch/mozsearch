#ifndef ErrorList_h__
#define ErrorList_h__

#include <stdint.h>

#define NS_ERROR_MODULE_BASE_OFFSET 69
#define NS_ERROR_MODULE_XPCOM 1

enum class nsresult : uint32_t
{
  NS_OK = 0x0,
  NS_ERROR_UNEXPECTED = 0x8000FFFF,
  NS_BINDING_SUCCEEDED = 0x0,
};

const nsresult
  NS_OK = nsresult::NS_OK,
  NS_ERROR_UNEXPECTED = nsresult::NS_ERROR_UNEXPECTED,
  NS_BINDING_SUCCEEDED = nsresult::NS_BINDING_SUCCEEDED;

#endif // ErrorList_h__
