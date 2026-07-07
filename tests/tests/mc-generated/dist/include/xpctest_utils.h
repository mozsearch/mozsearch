/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_utils.idl
 */

#ifndef __gen_xpctest_utils_h__
#define __gen_xpctest_utils_h__


#include "nsISupports.h"

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif

/* starting interface:    nsIXPCTestFunctionInterface */
#define NS_IXPCTESTFUNCTIONINTERFACE_IID_STR "d58a82ab-d8f7-4ca9-9273-b3290d42a0cf"

#define NS_IXPCTESTFUNCTIONINTERFACE_IID \
  {0xd58a82ab, 0xd8f7, 0x4ca9, \
    { 0x92, 0x73, 0xb3, 0x29, 0x0d, 0x42, 0xa0, 0xcf }}

class NS_NO_VTABLE nsIXPCTestFunctionInterface : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTFUNCTIONINTERFACE_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestFunctionInterface;

  /* string echo (in string arg); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD Echo(const char * arg, char * *_retval) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTFUNCTIONINTERFACE \
  NS_IMETHOD Echo(const char * arg, char * *_retval) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTFUNCTIONINTERFACE \
  nsresult Echo(const char * arg, char * *_retval); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTFUNCTIONINTERFACE(_to) \
  NS_IMETHOD Echo(const char * arg, char * *_retval) override { return _to Echo(arg, _retval); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTFUNCTIONINTERFACE(_to) \
  NS_IMETHOD Echo(const char * arg, char * *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->Echo(arg, _retval); } 


/* starting interface:    nsIXPCTestUtils */
#define NS_IXPCTESTUTILS_IID_STR "1e9cddeb-510d-449a-b152-3c1b5b31d41d"

#define NS_IXPCTESTUTILS_IID \
  {0x1e9cddeb, 0x510d, 0x449a, \
    { 0xb1, 0x52, 0x3c, 0x1b, 0x5b, 0x31, 0xd4, 0x1d }}

class NS_NO_VTABLE nsIXPCTestUtils : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTUTILS_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestUtils;

  /* nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD DoubleWrapFunction(nsIXPCTestFunctionInterface *f, nsIXPCTestFunctionInterface **_retval) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTUTILS \
  NS_IMETHOD DoubleWrapFunction(nsIXPCTestFunctionInterface *f, nsIXPCTestFunctionInterface **_retval) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTUTILS \
  nsresult DoubleWrapFunction(nsIXPCTestFunctionInterface *f, nsIXPCTestFunctionInterface **_retval); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTUTILS(_to) \
  NS_IMETHOD DoubleWrapFunction(nsIXPCTestFunctionInterface *f, nsIXPCTestFunctionInterface **_retval) override { return _to DoubleWrapFunction(f, _retval); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTUTILS(_to) \
  NS_IMETHOD DoubleWrapFunction(nsIXPCTestFunctionInterface *f, nsIXPCTestFunctionInterface **_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->DoubleWrapFunction(f, _retval); } 

typedef void *  Noncompat;


/* starting interface:    nsIXPCTestNotScriptable */
#define NS_IXPCTESTNOTSCRIPTABLE_IID_STR "ddf64cfb-668a-4571-a900-0fe2babb6249"

#define NS_IXPCTESTNOTSCRIPTABLE_IID \
  {0xddf64cfb, 0x668a, 0x4571, \
    { 0xa9, 0x00, 0x0f, 0xe2, 0xba, 0xbb, 0x62, 0x49 }}

class NS_NO_VTABLE nsIXPCTestNotScriptable : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTNOTSCRIPTABLE_IID)

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTNOTSCRIPTABLE \
  /* no methods! */

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTNOTSCRIPTABLE \
  /* no methods! */

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTNOTSCRIPTABLE(_to) \
  /* no methods! */

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTNOTSCRIPTABLE(_to) \
  /* no methods! */


/* starting interface:    nsIXPCTestTypeScript */
#define NS_IXPCTESTTYPESCRIPT_IID_STR "1bbfe703-c67d-4995-b061-564c8a1c39d7"

#define NS_IXPCTESTTYPESCRIPT_IID \
  {0x1bbfe703, 0xc67d, 0x4995, \
    { 0xb0, 0x61, 0x56, 0x4c, 0x8a, 0x1c, 0x39, 0xd7 }}

class NS_NO_VTABLE nsIXPCTestTypeScript : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTTYPESCRIPT_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestTypeScript;

  /* attribute long exposedProp; */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD GetExposedProp(int32_t *aExposedProp) = 0;
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD SetExposedProp(int32_t aExposedProp) = 0;

  /* void exposedMethod (in long arg); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD ExposedMethod(int32_t arg) = 0;

  /* [noscript] attribute Noncompat noncompatProp; */
  NS_IMETHOD GetNoncompatProp(Noncompat *aNoncompatProp) = 0;
  NS_IMETHOD SetNoncompatProp(Noncompat aNoncompatProp) = 0;

  /* [noscript] void noncompatMethod (in Noncompat arg); */
  NS_IMETHOD NoncompatMethod(Noncompat arg) = 0;

  /* [noscript] attribute long noscriptProp; */
  NS_IMETHOD GetNoscriptProp(int32_t *aNoscriptProp) = 0;
  NS_IMETHOD SetNoscriptProp(int32_t aNoscriptProp) = 0;

  /* [noscript] void noscriptMethod (in long arg); */
  NS_IMETHOD NoscriptMethod(int32_t arg) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTTYPESCRIPT \
  NS_IMETHOD GetExposedProp(int32_t *aExposedProp) override; \
  NS_IMETHOD SetExposedProp(int32_t aExposedProp) override; \
  NS_IMETHOD ExposedMethod(int32_t arg) override; \
  NS_IMETHOD GetNoncompatProp(Noncompat *aNoncompatProp) override; \
  NS_IMETHOD SetNoncompatProp(Noncompat aNoncompatProp) override; \
  NS_IMETHOD NoncompatMethod(Noncompat arg) override; \
  NS_IMETHOD GetNoscriptProp(int32_t *aNoscriptProp) override; \
  NS_IMETHOD SetNoscriptProp(int32_t aNoscriptProp) override; \
  NS_IMETHOD NoscriptMethod(int32_t arg) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTTYPESCRIPT \
  nsresult GetExposedProp(int32_t *aExposedProp); \
  nsresult SetExposedProp(int32_t aExposedProp); \
  nsresult ExposedMethod(int32_t arg); \
  nsresult GetNoncompatProp(Noncompat *aNoncompatProp); \
  nsresult SetNoncompatProp(Noncompat aNoncompatProp); \
  nsresult NoncompatMethod(Noncompat arg); \
  nsresult GetNoscriptProp(int32_t *aNoscriptProp); \
  nsresult SetNoscriptProp(int32_t aNoscriptProp); \
  nsresult NoscriptMethod(int32_t arg); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTTYPESCRIPT(_to) \
  NS_IMETHOD GetExposedProp(int32_t *aExposedProp) override { return _to GetExposedProp(aExposedProp); } \
  NS_IMETHOD SetExposedProp(int32_t aExposedProp) override { return _to SetExposedProp(aExposedProp); } \
  NS_IMETHOD ExposedMethod(int32_t arg) override { return _to ExposedMethod(arg); } \
  NS_IMETHOD GetNoncompatProp(Noncompat *aNoncompatProp) override { return _to GetNoncompatProp(aNoncompatProp); } \
  NS_IMETHOD SetNoncompatProp(Noncompat aNoncompatProp) override { return _to SetNoncompatProp(aNoncompatProp); } \
  NS_IMETHOD NoncompatMethod(Noncompat arg) override { return _to NoncompatMethod(arg); } \
  NS_IMETHOD GetNoscriptProp(int32_t *aNoscriptProp) override { return _to GetNoscriptProp(aNoscriptProp); } \
  NS_IMETHOD SetNoscriptProp(int32_t aNoscriptProp) override { return _to SetNoscriptProp(aNoscriptProp); } \
  NS_IMETHOD NoscriptMethod(int32_t arg) override { return _to NoscriptMethod(arg); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTTYPESCRIPT(_to) \
  NS_IMETHOD GetExposedProp(int32_t *aExposedProp) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetExposedProp(aExposedProp); } \
  NS_IMETHOD SetExposedProp(int32_t aExposedProp) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetExposedProp(aExposedProp); } \
  NS_IMETHOD ExposedMethod(int32_t arg) override { return !_to ? NS_ERROR_NULL_POINTER : _to->ExposedMethod(arg); } \
  NS_IMETHOD GetNoncompatProp(Noncompat *aNoncompatProp) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetNoncompatProp(aNoncompatProp); } \
  NS_IMETHOD SetNoncompatProp(Noncompat aNoncompatProp) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetNoncompatProp(aNoncompatProp); } \
  NS_IMETHOD NoncompatMethod(Noncompat arg) override { return !_to ? NS_ERROR_NULL_POINTER : _to->NoncompatMethod(arg); } \
  NS_IMETHOD GetNoscriptProp(int32_t *aNoscriptProp) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetNoscriptProp(aNoscriptProp); } \
  NS_IMETHOD SetNoscriptProp(int32_t aNoscriptProp) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetNoscriptProp(aNoscriptProp); } \
  NS_IMETHOD NoscriptMethod(int32_t arg) override { return !_to ? NS_ERROR_NULL_POINTER : _to->NoscriptMethod(arg); } 


#endif /* __gen_xpctest_utils_h__ */
