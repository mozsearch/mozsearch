/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_bug809674.idl
 */

#ifndef __gen_xpctest_bug809674_h__
#define __gen_xpctest_bug809674_h__


#ifndef __gen_nsISupports_h__
#include "nsISupports.h"
#endif

#include "js/Value.h"

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif

/* starting interface:    nsIXPCTestBug809674 */
#define NS_IXPCTESTBUG809674_IID_STR "2df46559-da21-49bf-b863-0d7b7bbcbc73"

#define NS_IXPCTESTBUG809674_IID \
  {0x2df46559, 0xda21, 0x49bf, \
    { 0xb8, 0x63, 0x0d, 0x7b, 0x7b, 0xbc, 0xbc, 0x73 }}

class NS_NO_VTABLE nsIXPCTestBug809674 : public nsISupports {
 public:

  NS_DECLARE_STATIC_IID_ACCESSOR(NS_IXPCTESTBUG809674_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestBug809674;

  /* [implicit_jscontext] unsigned long addArgs (in unsigned long x, in unsigned long y); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD AddArgs(uint32_t x, uint32_t y, JSContext* cx, uint32_t *_retval) = 0;

  /* [implicit_jscontext] unsigned long addSubMulArgs (in unsigned long x, in unsigned long y, out unsigned long subOut, out unsigned long mulOut); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD AddSubMulArgs(uint32_t x, uint32_t y, uint32_t *subOut, uint32_t *mulOut, JSContext* cx, uint32_t *_retval) = 0;

  /* [implicit_jscontext] jsval addVals (in jsval x, in jsval y); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD AddVals(JS::HandleValue x, JS::HandleValue y, JSContext* cx, JS::MutableHandleValue _retval) = 0;

  /* [implicit_jscontext] unsigned long methodNoArgs (); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD MethodNoArgs(JSContext* cx, uint32_t *_retval) = 0;

  /* [implicit_jscontext] void methodNoArgsNoRetVal (); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD MethodNoArgsNoRetVal(JSContext* cx) = 0;

  /* [implicit_jscontext] unsigned long addMany (in unsigned long x1, in unsigned long x2, in unsigned long x3, in unsigned long x4, in unsigned long x5, in unsigned long x6, in unsigned long x7, in unsigned long x8); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD AddMany(uint32_t x1, uint32_t x2, uint32_t x3, uint32_t x4, uint32_t x5, uint32_t x6, uint32_t x7, uint32_t x8, JSContext* cx, uint32_t *_retval) = 0;

  /* [implicit_jscontext] attribute jsval valProperty; */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD GetValProperty(JSContext* cx, JS::MutableHandleValue aValProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD SetValProperty(JSContext* cx, JS::HandleValue aValProperty) = 0;

  /* [implicit_jscontext] attribute unsigned long uintProperty; */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD GetUintProperty(JSContext* cx, uint32_t *aUintProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD SetUintProperty(JSContext* cx, uint32_t aUintProperty) = 0;

  /* [optional_argc] void methodWithOptionalArgc (); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD MethodWithOptionalArgc(uint8_t _argc) = 0;

};

  NS_DEFINE_STATIC_IID_ACCESSOR(nsIXPCTestBug809674, NS_IXPCTESTBUG809674_IID)

/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTBUG809674 \
  NS_IMETHOD AddArgs(uint32_t x, uint32_t y, JSContext* cx, uint32_t *_retval) override; \
  NS_IMETHOD AddSubMulArgs(uint32_t x, uint32_t y, uint32_t *subOut, uint32_t *mulOut, JSContext* cx, uint32_t *_retval) override; \
  NS_IMETHOD AddVals(JS::HandleValue x, JS::HandleValue y, JSContext* cx, JS::MutableHandleValue _retval) override; \
  NS_IMETHOD MethodNoArgs(JSContext* cx, uint32_t *_retval) override; \
  NS_IMETHOD MethodNoArgsNoRetVal(JSContext* cx) override; \
  NS_IMETHOD AddMany(uint32_t x1, uint32_t x2, uint32_t x3, uint32_t x4, uint32_t x5, uint32_t x6, uint32_t x7, uint32_t x8, JSContext* cx, uint32_t *_retval) override; \
  NS_IMETHOD GetValProperty(JSContext* cx, JS::MutableHandleValue aValProperty) override; \
  NS_IMETHOD SetValProperty(JSContext* cx, JS::HandleValue aValProperty) override; \
  NS_IMETHOD GetUintProperty(JSContext* cx, uint32_t *aUintProperty) override; \
  NS_IMETHOD SetUintProperty(JSContext* cx, uint32_t aUintProperty) override; \
  NS_IMETHOD MethodWithOptionalArgc(uint8_t _argc) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTBUG809674 \
  nsresult AddArgs(uint32_t x, uint32_t y, JSContext* cx, uint32_t *_retval); \
  nsresult AddSubMulArgs(uint32_t x, uint32_t y, uint32_t *subOut, uint32_t *mulOut, JSContext* cx, uint32_t *_retval); \
  nsresult AddVals(JS::HandleValue x, JS::HandleValue y, JSContext* cx, JS::MutableHandleValue _retval); \
  nsresult MethodNoArgs(JSContext* cx, uint32_t *_retval); \
  nsresult MethodNoArgsNoRetVal(JSContext* cx); \
  nsresult AddMany(uint32_t x1, uint32_t x2, uint32_t x3, uint32_t x4, uint32_t x5, uint32_t x6, uint32_t x7, uint32_t x8, JSContext* cx, uint32_t *_retval); \
  nsresult GetValProperty(JSContext* cx, JS::MutableHandleValue aValProperty); \
  nsresult SetValProperty(JSContext* cx, JS::HandleValue aValProperty); \
  nsresult GetUintProperty(JSContext* cx, uint32_t *aUintProperty); \
  nsresult SetUintProperty(JSContext* cx, uint32_t aUintProperty); \
  nsresult MethodWithOptionalArgc(uint8_t _argc); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTBUG809674(_to) \
  NS_IMETHOD AddArgs(uint32_t x, uint32_t y, JSContext* cx, uint32_t *_retval) override { return _to AddArgs(x, y, cx, _retval); } \
  NS_IMETHOD AddSubMulArgs(uint32_t x, uint32_t y, uint32_t *subOut, uint32_t *mulOut, JSContext* cx, uint32_t *_retval) override { return _to AddSubMulArgs(x, y, subOut, mulOut, cx, _retval); } \
  NS_IMETHOD AddVals(JS::HandleValue x, JS::HandleValue y, JSContext* cx, JS::MutableHandleValue _retval) override { return _to AddVals(x, y, cx, _retval); } \
  NS_IMETHOD MethodNoArgs(JSContext* cx, uint32_t *_retval) override { return _to MethodNoArgs(cx, _retval); } \
  NS_IMETHOD MethodNoArgsNoRetVal(JSContext* cx) override { return _to MethodNoArgsNoRetVal(cx); } \
  NS_IMETHOD AddMany(uint32_t x1, uint32_t x2, uint32_t x3, uint32_t x4, uint32_t x5, uint32_t x6, uint32_t x7, uint32_t x8, JSContext* cx, uint32_t *_retval) override { return _to AddMany(x1, x2, x3, x4, x5, x6, x7, x8, cx, _retval); } \
  NS_IMETHOD GetValProperty(JSContext* cx, JS::MutableHandleValue aValProperty) override { return _to GetValProperty(cx, aValProperty); } \
  NS_IMETHOD SetValProperty(JSContext* cx, JS::HandleValue aValProperty) override { return _to SetValProperty(cx, aValProperty); } \
  NS_IMETHOD GetUintProperty(JSContext* cx, uint32_t *aUintProperty) override { return _to GetUintProperty(cx, aUintProperty); } \
  NS_IMETHOD SetUintProperty(JSContext* cx, uint32_t aUintProperty) override { return _to SetUintProperty(cx, aUintProperty); } \
  NS_IMETHOD MethodWithOptionalArgc(uint8_t _argc) override { return _to MethodWithOptionalArgc(_argc); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTBUG809674(_to) \
  NS_IMETHOD AddArgs(uint32_t x, uint32_t y, JSContext* cx, uint32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->AddArgs(x, y, cx, _retval); } \
  NS_IMETHOD AddSubMulArgs(uint32_t x, uint32_t y, uint32_t *subOut, uint32_t *mulOut, JSContext* cx, uint32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->AddSubMulArgs(x, y, subOut, mulOut, cx, _retval); } \
  NS_IMETHOD AddVals(JS::HandleValue x, JS::HandleValue y, JSContext* cx, JS::MutableHandleValue _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->AddVals(x, y, cx, _retval); } \
  NS_IMETHOD MethodNoArgs(JSContext* cx, uint32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->MethodNoArgs(cx, _retval); } \
  NS_IMETHOD MethodNoArgsNoRetVal(JSContext* cx) override { return !_to ? NS_ERROR_NULL_POINTER : _to->MethodNoArgsNoRetVal(cx); } \
  NS_IMETHOD AddMany(uint32_t x1, uint32_t x2, uint32_t x3, uint32_t x4, uint32_t x5, uint32_t x6, uint32_t x7, uint32_t x8, JSContext* cx, uint32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->AddMany(x1, x2, x3, x4, x5, x6, x7, x8, cx, _retval); } \
  NS_IMETHOD GetValProperty(JSContext* cx, JS::MutableHandleValue aValProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetValProperty(cx, aValProperty); } \
  NS_IMETHOD SetValProperty(JSContext* cx, JS::HandleValue aValProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetValProperty(cx, aValProperty); } \
  NS_IMETHOD GetUintProperty(JSContext* cx, uint32_t *aUintProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetUintProperty(cx, aUintProperty); } \
  NS_IMETHOD SetUintProperty(JSContext* cx, uint32_t aUintProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetUintProperty(cx, aUintProperty); } \
  NS_IMETHOD MethodWithOptionalArgc(uint8_t _argc) override { return !_to ? NS_ERROR_NULL_POINTER : _to->MethodWithOptionalArgc(_argc); } 


#endif /* __gen_xpctest_bug809674_h__ */
