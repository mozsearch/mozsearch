/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_returncode.idl
 */

#ifndef __gen_xpctest_returncode_h__
#define __gen_xpctest_returncode_h__


#include "nsISupports.h"

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif

/* starting interface:    nsIXPCTestReturnCodeParent */
#define NS_IXPCTESTRETURNCODEPARENT_IID_STR "479e4532-95cf-48b8-a99b-8a5881e47138"

#define NS_IXPCTESTRETURNCODEPARENT_IID \
  {0x479e4532, 0x95cf, 0x48b8, \
    { 0xa9, 0x9b, 0x8a, 0x58, 0x81, 0xe4, 0x71, 0x38 }}

class NS_NO_VTABLE MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestReturnCodeParent) nsIXPCTestReturnCodeParent : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTRETURNCODEPARENT_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestReturnCodeParent;

  /* nsresult callChild (in long childBehavior); */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeParent_callChild) NS_IMETHOD CallChild(int32_t childBehavior, nsresult *_retval) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTRETURNCODEPARENT \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeParent_callChild) NS_IMETHOD CallChild(int32_t childBehavior, nsresult *_retval) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTRETURNCODEPARENT \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeParent_callChild) nsresult CallChild(int32_t childBehavior, nsresult *_retval); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTRETURNCODEPARENT(_to) \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeParent_callChild) NS_IMETHOD CallChild(int32_t childBehavior, nsresult *_retval) override { return _to CallChild(childBehavior, _retval); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTRETURNCODEPARENT(_to) \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeParent_callChild) NS_IMETHOD CallChild(int32_t childBehavior, nsresult *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->CallChild(childBehavior, _retval); } 


/* starting interface:    nsIXPCTestReturnCodeChild */
#define NS_IXPCTESTRETURNCODECHILD_IID_STR "672cfd34-1fd1-455d-9901-d879fa6fdb95"

#define NS_IXPCTESTRETURNCODECHILD_IID \
  {0x672cfd34, 0x1fd1, 0x455d, \
    { 0x99, 0x01, 0xd8, 0x79, 0xfa, 0x6f, 0xdb, 0x95 }}

class NS_NO_VTABLE MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestReturnCodeChild) nsIXPCTestReturnCodeChild : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTRETURNCODECHILD_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestReturnCodeChild;

  /* void doIt (in long behavior); */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeChild_doIt) NS_IMETHOD DoIt(int32_t behavior) = 0;

  enum {
    CHILD_SHOULD_THROW MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestReturnCodeChild_CHILD_SHOULD_THROW) = 0,
    CHILD_SHOULD_RETURN_SUCCESS MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestReturnCodeChild_CHILD_SHOULD_RETURN_SUCCESS) = 1,
    CHILD_SHOULD_RETURN_RESULTCODE MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestReturnCodeChild_CHILD_SHOULD_RETURN_RESULTCODE) = 2,
    CHILD_SHOULD_NEST_RESULTCODES MOZ_BINDING(binding_to, idl, const, XPIDL_nsIXPCTestReturnCodeChild_CHILD_SHOULD_NEST_RESULTCODES) = 3
  };

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTRETURNCODECHILD \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeChild_doIt) NS_IMETHOD DoIt(int32_t behavior) override; \

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTRETURNCODECHILD \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeChild_doIt) nsresult DoIt(int32_t behavior); \

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTRETURNCODECHILD(_to) \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeChild_doIt) NS_IMETHOD DoIt(int32_t behavior) override { return _to DoIt(behavior); } \

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTRETURNCODECHILD(_to) \
  MOZ_BINDING(binding_to, idl, method, XPIDL_nsIXPCTestReturnCodeChild_doIt) NS_IMETHOD DoIt(int32_t behavior) override { return !_to ? NS_ERROR_NULL_POINTER : _to->DoIt(behavior); } \


#endif /* __gen_xpctest_returncode_h__ */
