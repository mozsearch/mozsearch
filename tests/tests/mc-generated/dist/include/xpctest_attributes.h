/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_attributes.idl
 */

#ifndef __gen_xpctest_attributes_h__
#define __gen_xpctest_attributes_h__


#include "nsISupports.h"

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif

/* starting interface:    nsIXPCTestObjectReadOnly */
#define NS_IXPCTESTOBJECTREADONLY_IID_STR "42fbd9f6-b12d-47ef-b7a1-02d73c11fe53"

#define NS_IXPCTESTOBJECTREADONLY_IID \
  {0x42fbd9f6, 0xb12d, 0x47ef, \
    { 0xb7, 0xa1, 0x02, 0xd7, 0x3c, 0x11, 0xfe, 0x53 }}

class NS_NO_VTABLE MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestObjectReadOnly) nsIXPCTestObjectReadOnly : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTOBJECTREADONLY_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestObjectReadOnly;

  /* readonly attribute string strReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_strReadOnly) NS_IMETHOD GetStrReadOnly(char * *aStrReadOnly) = 0;

  /* readonly attribute boolean boolReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_boolReadOnly) NS_IMETHOD GetBoolReadOnly(bool *aBoolReadOnly) = 0;

  /* readonly attribute short shortReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_shortReadOnly) NS_IMETHOD GetShortReadOnly(int16_t *aShortReadOnly) = 0;

  /* readonly attribute long longReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_longReadOnly) NS_IMETHOD GetLongReadOnly(int32_t *aLongReadOnly) = 0;

  /* readonly attribute float floatReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_floatReadOnly) NS_IMETHOD GetFloatReadOnly(float *aFloatReadOnly) = 0;

  /* readonly attribute char charReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_charReadOnly) NS_IMETHOD GetCharReadOnly(char *aCharReadOnly) = 0;

  /* readonly attribute PRTime timeReadOnly; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_timeReadOnly) NS_IMETHOD GetTimeReadOnly(PRTime *aTimeReadOnly) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTOBJECTREADONLY \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_strReadOnly) NS_IMETHOD GetStrReadOnly(char * *aStrReadOnly) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_boolReadOnly) NS_IMETHOD GetBoolReadOnly(bool *aBoolReadOnly) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_shortReadOnly) NS_IMETHOD GetShortReadOnly(int16_t *aShortReadOnly) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_longReadOnly) NS_IMETHOD GetLongReadOnly(int32_t *aLongReadOnly) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_floatReadOnly) NS_IMETHOD GetFloatReadOnly(float *aFloatReadOnly) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_charReadOnly) NS_IMETHOD GetCharReadOnly(char *aCharReadOnly) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_timeReadOnly) NS_IMETHOD GetTimeReadOnly(PRTime *aTimeReadOnly) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTOBJECTREADONLY \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_strReadOnly) nsresult GetStrReadOnly(char * *aStrReadOnly); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_boolReadOnly) nsresult GetBoolReadOnly(bool *aBoolReadOnly); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_shortReadOnly) nsresult GetShortReadOnly(int16_t *aShortReadOnly); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_longReadOnly) nsresult GetLongReadOnly(int32_t *aLongReadOnly); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_floatReadOnly) nsresult GetFloatReadOnly(float *aFloatReadOnly); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_charReadOnly) nsresult GetCharReadOnly(char *aCharReadOnly); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_timeReadOnly) nsresult GetTimeReadOnly(PRTime *aTimeReadOnly); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTOBJECTREADONLY(_to) \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_strReadOnly) NS_IMETHOD GetStrReadOnly(char * *aStrReadOnly) override { return _to GetStrReadOnly(aStrReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_boolReadOnly) NS_IMETHOD GetBoolReadOnly(bool *aBoolReadOnly) override { return _to GetBoolReadOnly(aBoolReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_shortReadOnly) NS_IMETHOD GetShortReadOnly(int16_t *aShortReadOnly) override { return _to GetShortReadOnly(aShortReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_longReadOnly) NS_IMETHOD GetLongReadOnly(int32_t *aLongReadOnly) override { return _to GetLongReadOnly(aLongReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_floatReadOnly) NS_IMETHOD GetFloatReadOnly(float *aFloatReadOnly) override { return _to GetFloatReadOnly(aFloatReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_charReadOnly) NS_IMETHOD GetCharReadOnly(char *aCharReadOnly) override { return _to GetCharReadOnly(aCharReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_timeReadOnly) NS_IMETHOD GetTimeReadOnly(PRTime *aTimeReadOnly) override { return _to GetTimeReadOnly(aTimeReadOnly); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTOBJECTREADONLY(_to) \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_strReadOnly) NS_IMETHOD GetStrReadOnly(char * *aStrReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetStrReadOnly(aStrReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_boolReadOnly) NS_IMETHOD GetBoolReadOnly(bool *aBoolReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetBoolReadOnly(aBoolReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_shortReadOnly) NS_IMETHOD GetShortReadOnly(int16_t *aShortReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetShortReadOnly(aShortReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_longReadOnly) NS_IMETHOD GetLongReadOnly(int32_t *aLongReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetLongReadOnly(aLongReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_floatReadOnly) NS_IMETHOD GetFloatReadOnly(float *aFloatReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetFloatReadOnly(aFloatReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_charReadOnly) NS_IMETHOD GetCharReadOnly(char *aCharReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetCharReadOnly(aCharReadOnly); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadOnly_timeReadOnly) NS_IMETHOD GetTimeReadOnly(PRTime *aTimeReadOnly) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetTimeReadOnly(aTimeReadOnly); } 


/* starting interface:    nsIXPCTestObjectReadWrite */
#define NS_IXPCTESTOBJECTREADWRITE_IID_STR "f07529b0-a479-4954-aba5-ab3142c6b1cb"

#define NS_IXPCTESTOBJECTREADWRITE_IID \
  {0xf07529b0, 0xa479, 0x4954, \
    { 0xab, 0xa5, 0xab, 0x31, 0x42, 0xc6, 0xb1, 0xcb }}

class NS_NO_VTABLE MOZ_BINDING(binding_to, idl, class, XPIDL_nsIXPCTestObjectReadWrite) nsIXPCTestObjectReadWrite : public nsISupports {
 public:

  NS_INLINE_DECL_STATIC_IID(NS_IXPCTESTOBJECTREADWRITE_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestObjectReadWrite;

  /* attribute string stringProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD GetStringProperty(char * *aStringProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD SetStringProperty(const char * aStringProperty) = 0;

  /* attribute boolean booleanProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD GetBooleanProperty(bool *aBooleanProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD SetBooleanProperty(bool aBooleanProperty) = 0;

  /* attribute short shortProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD GetShortProperty(int16_t *aShortProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD SetShortProperty(int16_t aShortProperty) = 0;

  /* attribute long longProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD GetLongProperty(int32_t *aLongProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD SetLongProperty(int32_t aLongProperty) = 0;

  /* attribute float floatProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD GetFloatProperty(float *aFloatProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD SetFloatProperty(float aFloatProperty) = 0;

  /* attribute char charProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD GetCharProperty(char *aCharProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD SetCharProperty(char aCharProperty) = 0;

  /* attribute PRTime timeProperty; */
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD GetTimeProperty(PRTime *aTimeProperty) = 0;
  JS_HAZ_CAN_RUN_SCRIPT MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD SetTimeProperty(PRTime aTimeProperty) = 0;

};


/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTOBJECTREADWRITE \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD GetStringProperty(char * *aStringProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD SetStringProperty(const char * aStringProperty) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD GetBooleanProperty(bool *aBooleanProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD SetBooleanProperty(bool aBooleanProperty) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD GetShortProperty(int16_t *aShortProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD SetShortProperty(int16_t aShortProperty) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD GetLongProperty(int32_t *aLongProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD SetLongProperty(int32_t aLongProperty) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD GetFloatProperty(float *aFloatProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD SetFloatProperty(float aFloatProperty) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD GetCharProperty(char *aCharProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD SetCharProperty(char aCharProperty) override; \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD GetTimeProperty(PRTime *aTimeProperty) override; \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD SetTimeProperty(PRTime aTimeProperty) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTOBJECTREADWRITE \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) nsresult GetStringProperty(char * *aStringProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) nsresult SetStringProperty(const char * aStringProperty); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) nsresult GetBooleanProperty(bool *aBooleanProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) nsresult SetBooleanProperty(bool aBooleanProperty); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) nsresult GetShortProperty(int16_t *aShortProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) nsresult SetShortProperty(int16_t aShortProperty); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) nsresult GetLongProperty(int32_t *aLongProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) nsresult SetLongProperty(int32_t aLongProperty); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) nsresult GetFloatProperty(float *aFloatProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) nsresult SetFloatProperty(float aFloatProperty); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) nsresult GetCharProperty(char *aCharProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) nsresult SetCharProperty(char aCharProperty); \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) nsresult GetTimeProperty(PRTime *aTimeProperty); \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) nsresult SetTimeProperty(PRTime aTimeProperty); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTOBJECTREADWRITE(_to) \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD GetStringProperty(char * *aStringProperty) override { return _to GetStringProperty(aStringProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD SetStringProperty(const char * aStringProperty) override { return _to SetStringProperty(aStringProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD GetBooleanProperty(bool *aBooleanProperty) override { return _to GetBooleanProperty(aBooleanProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD SetBooleanProperty(bool aBooleanProperty) override { return _to SetBooleanProperty(aBooleanProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD GetShortProperty(int16_t *aShortProperty) override { return _to GetShortProperty(aShortProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD SetShortProperty(int16_t aShortProperty) override { return _to SetShortProperty(aShortProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD GetLongProperty(int32_t *aLongProperty) override { return _to GetLongProperty(aLongProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD SetLongProperty(int32_t aLongProperty) override { return _to SetLongProperty(aLongProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD GetFloatProperty(float *aFloatProperty) override { return _to GetFloatProperty(aFloatProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD SetFloatProperty(float aFloatProperty) override { return _to SetFloatProperty(aFloatProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD GetCharProperty(char *aCharProperty) override { return _to GetCharProperty(aCharProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD SetCharProperty(char aCharProperty) override { return _to SetCharProperty(aCharProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD GetTimeProperty(PRTime *aTimeProperty) override { return _to GetTimeProperty(aTimeProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD SetTimeProperty(PRTime aTimeProperty) override { return _to SetTimeProperty(aTimeProperty); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTOBJECTREADWRITE(_to) \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD GetStringProperty(char * *aStringProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetStringProperty(aStringProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_stringProperty) NS_IMETHOD SetStringProperty(const char * aStringProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetStringProperty(aStringProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD GetBooleanProperty(bool *aBooleanProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetBooleanProperty(aBooleanProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_booleanProperty) NS_IMETHOD SetBooleanProperty(bool aBooleanProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetBooleanProperty(aBooleanProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD GetShortProperty(int16_t *aShortProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetShortProperty(aShortProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_shortProperty) NS_IMETHOD SetShortProperty(int16_t aShortProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetShortProperty(aShortProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD GetLongProperty(int32_t *aLongProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetLongProperty(aLongProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_longProperty) NS_IMETHOD SetLongProperty(int32_t aLongProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetLongProperty(aLongProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD GetFloatProperty(float *aFloatProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetFloatProperty(aFloatProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_floatProperty) NS_IMETHOD SetFloatProperty(float aFloatProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetFloatProperty(aFloatProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD GetCharProperty(char *aCharProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetCharProperty(aCharProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_charProperty) NS_IMETHOD SetCharProperty(char aCharProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetCharProperty(aCharProperty); } \
  MOZ_BINDING(binding_to, idl, getter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD GetTimeProperty(PRTime *aTimeProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->GetTimeProperty(aTimeProperty); } \
  MOZ_BINDING(binding_to, idl, setter, XPIDL_nsIXPCTestObjectReadWrite_timeProperty) NS_IMETHOD SetTimeProperty(PRTime aTimeProperty) override { return !_to ? NS_ERROR_NULL_POINTER : _to->SetTimeProperty(aTimeProperty); } 


#endif /* __gen_xpctest_attributes_h__ */
