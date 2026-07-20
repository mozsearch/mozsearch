// This file is just a stub to let xpctest_params.h compile!
//
// As it's the first file it includes and is the most obvious places to look for
// dummying definitions, that's where we put them.
//
// No attempt is currently made to actually resemble the real nsISupports
// infrastructure!

#ifndef nsISupports_h__
#define nsISupports_h__

#include <stdint.h>

#define NS_NO_VTABLE
#define NS_DECLARE_STATIC_IID_ACCESSOR(foo)
#define JS_HAZ_CAN_RUN_SCRIPT
#define NS_IMETHOD virtual uint32_t
#define NS_DEFINE_STATIC_IID_ACCESSOR(foo, bar)
#define NS_INLINE_DECL_STATIC_IID(the_iid) static constexpr nsIID kIID = the_iid;
#define MOZ_BINDING(direction, language, kind, symbol) __attribute__((annotate(#direction, #language, #kind, #symbol)))

class nsISupports {};

class nsAString;
class nsString;

class nsACString;
class nsCString;

struct nsID {
  uint32_t m0;
  uint16_t m1;
  uint16_t m2;
  uint8_t m3[8];
};
typedef nsID nsIID;

template <class E>
class RefPtr {};

struct PRTime;

#endif  // nsISupports_h__
