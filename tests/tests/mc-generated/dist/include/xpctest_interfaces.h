/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_interfaces.idl
 */

#ifndef __gen_xpctest_interfaces_h__
#define __gen_xpctest_interfaces_h__


#ifndef __gen_nsISupports_h__
#include "nsISupports.h"
#endif

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif

/* starting interface:    nsIXPCTestInterfaceA */
#define NS_IXPCTESTINTERFACEA_IID_STR "3c8fd2f5-970c-42c6-b5dd-cda1c16dcfd8"

#define NS_IXPCTESTINTERFACEA_IID \
  {0x3c8fd2f5, 0x970c, 0x42c6, \
    { 0xb5, 0xdd, 0xcd, 0xa1, 0xc1, 0x6d, 0xcf, 0xd8 }}

class NS_NO_VTABLE nsIXPCTestInterfaceA : public nsISupports {
 public:

  NS_DECLARE_STATIC_IID_ACCESSOR(NS_IXPCTESTINTERFACEA_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestInterfaceA;

  /* attribute string name; */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD GetName(char * *aName) = 0;
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD SetName(const char * aName) = 0;

};

  NS_DEFINE_STATIC_IID_ACCESSOR(nsIXPCTestInterfaceA, NS_IXPCTESTINTERFACEA_IID)

/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTINTERFACEA \
  NS_IMETHOD GetName(char * *aName) override; \
  NS_IMETHOD SetName(const char * aName) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTINTERFACEA \
  nsresult GetName(char * *aName); \
  nsresult SetName(const char * aName); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTINTERFACEA(_to) \
  NS_IMETHOD GetName(char * *aName) override { return _to GetName(aName); } \
  NS_IMETHOD SetName(const char * aName) override { return _to SetName(aName); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTINTERFACEA(_to) \
  NS_IMETHOD GetName(char * *aName) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetName(aName); } \
  NS_IMETHOD SetName(const char * aName) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetName(aName); } 


/* starting interface:    nsIXPCTestInterfaceB */
#define NS_IXPCTESTINTERFACEB_IID_STR "ff528c3a-2410-46de-acaa-449aa6403a33"

#define NS_IXPCTESTINTERFACEB_IID \
  {0xff528c3a, 0x2410, 0x46de, \
    { 0xac, 0xaa, 0x44, 0x9a, 0xa6, 0x40, 0x3a, 0x33 }}

class NS_NO_VTABLE nsIXPCTestInterfaceB : public nsISupports {
 public:

  NS_DECLARE_STATIC_IID_ACCESSOR(NS_IXPCTESTINTERFACEB_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestInterfaceB;

  /* attribute string name; */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD GetName(char * *aName) = 0;
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD SetName(const char * aName) = 0;

};

  NS_DEFINE_STATIC_IID_ACCESSOR(nsIXPCTestInterfaceB, NS_IXPCTESTINTERFACEB_IID)

/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTINTERFACEB \
  NS_IMETHOD GetName(char * *aName) override; \
  NS_IMETHOD SetName(const char * aName) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTINTERFACEB \
  nsresult GetName(char * *aName); \
  nsresult SetName(const char * aName); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTINTERFACEB(_to) \
  NS_IMETHOD GetName(char * *aName) override { return _to GetName(aName); } \
  NS_IMETHOD SetName(const char * aName) override { return _to SetName(aName); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTINTERFACEB(_to) \
  NS_IMETHOD GetName(char * *aName) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetName(aName); } \
  NS_IMETHOD SetName(const char * aName) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetName(aName); } 


/* starting interface:    nsIXPCTestInterfaceC */
#define NS_IXPCTESTINTERFACEC_IID_STR "401cf1b4-355b-4cee-b7b3-c7973aee49bd"

#define NS_IXPCTESTINTERFACEC_IID \
  {0x401cf1b4, 0x355b, 0x4cee, \
    { 0xb7, 0xb3, 0xc7, 0x97, 0x3a, 0xee, 0x49, 0xbd }}

class NS_NO_VTABLE nsIXPCTestInterfaceC : public nsISupports {
 public:

  NS_DECLARE_STATIC_IID_ACCESSOR(NS_IXPCTESTINTERFACEC_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestInterfaceC;

  /* attribute long someInteger; */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD GetSomeInteger(int32_t *aSomeInteger) = 0;
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD SetSomeInteger(int32_t aSomeInteger) = 0;

};

  NS_DEFINE_STATIC_IID_ACCESSOR(nsIXPCTestInterfaceC, NS_IXPCTESTINTERFACEC_IID)

/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTINTERFACEC \
  NS_IMETHOD GetSomeInteger(int32_t *aSomeInteger) override; \
  NS_IMETHOD SetSomeInteger(int32_t aSomeInteger) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTINTERFACEC \
  nsresult GetSomeInteger(int32_t *aSomeInteger); \
  nsresult SetSomeInteger(int32_t aSomeInteger); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTINTERFACEC(_to) \
  NS_IMETHOD GetSomeInteger(int32_t *aSomeInteger) override { return _to GetSomeInteger(aSomeInteger); } \
  NS_IMETHOD SetSomeInteger(int32_t aSomeInteger) override { return _to SetSomeInteger(aSomeInteger); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTINTERFACEC(_to) \
  NS_IMETHOD GetSomeInteger(int32_t *aSomeInteger) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetSomeInteger(aSomeInteger); } \
  NS_IMETHOD SetSomeInteger(int32_t aSomeInteger) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetSomeInteger(aSomeInteger); } 


#endif /* __gen_xpctest_interfaces_h__ */
