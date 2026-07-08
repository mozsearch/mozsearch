/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_cenums.idl
 */

#ifndef __gen_xpctest_cenums_h__
#define __gen_xpctest_cenums_h__


#include "nsISupports.h"

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif

/* starting interface:    nsIXPCTestCEnums */
#define NS_IXPCTESTCENUMS_IID_STR "6a2f918e-cda2-11e8-bc9a-a34c716d1f2a"

#define NS_IXPCTESTCENUMS_IID \
  {0x6a2f918e, 0xcda2, 0x11e8, \
    { 0xbc, 0x9a, 0xa3, 0x4c, 0x71, 0x6d, 0x1f, 0x2a }}

class NS_NO_VTABLE MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestCEnums) nsIXPCTestCEnums : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTCENUMS_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestCEnums;

  enum {
    testConst MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testConst) = 1
  };

  enum MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestCEnums_testFlagsExplicit) testFlagsExplicit : uint8_t {
    shouldBe1Explicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsExplicit_shouldBe1Explicit) = 1,
    shouldBe2Explicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsExplicit_shouldBe2Explicit) = 2,
    shouldBe4Explicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsExplicit_shouldBe4Explicit) = 4,
    shouldBe8Explicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsExplicit_shouldBe8Explicit) = 8,
    shouldBe12Explicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsExplicit_shouldBe12Explicit) = 12,
  };

  enum MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestCEnums_testFlagsImplicit) testFlagsImplicit : uint8_t {
    shouldBe0Implicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe0Implicit) = 0,
    shouldBe1Implicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe1Implicit) = 1,
    shouldBe2Implicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe2Implicit) = 2,
    shouldBe3Implicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe3Implicit) = 3,
    shouldBe5Implicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe5Implicit) = 5,
    shouldBe6Implicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe6Implicit) = 6,
    shouldBe2AgainImplicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe2AgainImplicit) = 2,
    shouldBe3AgainImplicit MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestCEnums_testFlagsImplicit_shouldBe3AgainImplicit) = 3,
  };

  /* void testCEnumInput (in nsIXPCTestCEnums_testFlagsExplicit abc); */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumInput) NS_IMETHOD TestCEnumInput(nsIXPCTestCEnums::testFlagsExplicit abc) = 0;

  /* nsIXPCTestCEnums_testFlagsExplicit testCEnumOutput (); */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumOutput) NS_IMETHOD TestCEnumOutput(nsIXPCTestCEnums::testFlagsExplicit *_retval) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTCENUMS \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumInput) NS_IMETHOD TestCEnumInput(nsIXPCTestCEnums::testFlagsExplicit abc) override; \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumOutput) NS_IMETHOD TestCEnumOutput(nsIXPCTestCEnums::testFlagsExplicit *_retval) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTCENUMS \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumInput) nsresult TestCEnumInput(nsIXPCTestCEnums::testFlagsExplicit abc); \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumOutput) nsresult TestCEnumOutput(nsIXPCTestCEnums::testFlagsExplicit *_retval); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTCENUMS(_to) \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumInput) NS_IMETHOD TestCEnumInput(nsIXPCTestCEnums::testFlagsExplicit abc) override { return _to TestCEnumInput(abc); } \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumOutput) NS_IMETHOD TestCEnumOutput(nsIXPCTestCEnums::testFlagsExplicit *_retval) override { return _to TestCEnumOutput(_retval); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTCENUMS(_to) \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumInput) NS_IMETHOD TestCEnumInput(nsIXPCTestCEnums::testFlagsExplicit abc) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestCEnumInput(abc); } \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestCEnums_testCEnumOutput) NS_IMETHOD TestCEnumOutput(nsIXPCTestCEnums::testFlagsExplicit *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestCEnumOutput(_retval); } 


#endif /* __gen_xpctest_cenums_h__ */
