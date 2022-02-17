/*
 * DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_params.idl
 */

#ifndef __gen_xpctest_params_h__
#define __gen_xpctest_params_h__


#ifndef __gen_nsISupports_h__
#include "nsISupports.h"
#endif

#ifndef __gen_nsTArray_h__
#include "nsTArray.h"
#endif

#include "js/Value.h"

#include "js/GCAnnotations.h"

/* For IDL files that don't want to include root IDL files. */
#ifndef NS_NO_VTABLE
#define NS_NO_VTABLE
#endif
class nsIURI; /* forward declaration */

class nsIXPCTestInterfaceA; /* forward declaration */

class nsIXPCTestInterfaceB; /* forward declaration */


/* starting interface:    nsIXPCTestParams */
#define NS_IXPCTESTPARAMS_IID_STR "812145c7-9fcc-425e-a878-36ad1b7730b7"

#define NS_IXPCTESTPARAMS_IID \
  {0x812145c7, 0x9fcc, 0x425e, \
    { 0xa8, 0x78, 0x36, 0xad, 0x1b, 0x77, 0x30, 0xb7 }}

class NS_NO_VTABLE nsIXPCTestParams : public nsISupports {
 public:

  NS_DECLARE_STATIC_IID_ACCESSOR(NS_IXPCTESTPARAMS_IID)

  /* Used by ToJSValue to check which scriptable interface is implemented. */
  using ScriptableInterfaceType = nsIXPCTestParams;

  /* boolean testBoolean (in boolean a, inout boolean b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestBoolean(bool a, bool *b, bool *_retval) = 0;

  /* octet testOctet (in octet a, inout octet b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestOctet(uint8_t a, uint8_t *b, uint8_t *_retval) = 0;

  /* short testShort (in short a, inout short b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestShort(int16_t a, int16_t *b, int16_t *_retval) = 0;

  /* long testLong (in long a, inout long b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestLong(int32_t a, int32_t *b, int32_t *_retval) = 0;

  /* long long testLongLong (in long long a, inout long long b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestLongLong(int64_t a, int64_t *b, int64_t *_retval) = 0;

  /* unsigned short testUnsignedShort (in unsigned short a, inout unsigned short b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestUnsignedShort(uint16_t a, uint16_t *b, uint16_t *_retval) = 0;

  /* unsigned long testUnsignedLong (in unsigned long a, inout unsigned long b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestUnsignedLong(uint32_t a, uint32_t *b, uint32_t *_retval) = 0;

  /* unsigned long long testUnsignedLongLong (in unsigned long long a, inout unsigned long long b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestUnsignedLongLong(uint64_t a, uint64_t *b, uint64_t *_retval) = 0;

  /* float testFloat (in float a, inout float b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestFloat(float a, float *b, float *_retval) = 0;

  /* double testDouble (in double a, inout float b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestDouble(double a, float *b, double *_retval) = 0;

  /* char testChar (in char a, inout char b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestChar(char a, char *b, char *_retval) = 0;

  /* string testString (in string a, inout string b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestString(const char * a, char * *b, char * *_retval) = 0;

  /* wchar testWchar (in wchar a, inout wchar b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestWchar(char16_t a, char16_t *b, char16_t *_retval) = 0;

  /* wstring testWstring (in wstring a, inout wstring b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestWstring(const char16_t * a, char16_t * *b, char16_t * *_retval) = 0;

  /* AString testAString (in AString a, inout AString b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestAString(const nsAString& a, nsAString& b, nsAString& _retval) = 0;

  /* AUTF8String testAUTF8String (in AUTF8String a, inout AUTF8String b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestAUTF8String(const nsACString& a, nsACString& b, nsACString& _retval) = 0;

  /* ACString testACString (in ACString a, inout ACString b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestACString(const nsACString& a, nsACString& b, nsACString& _retval) = 0;

  /* jsval testJsval (in jsval a, inout jsval b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestJsval(JS::HandleValue a, JS::MutableHandleValue b, JS::MutableHandleValue _retval) = 0;

  /* Array<short> testShortSequence (in Array<short> a, inout Array<short> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestShortSequence(const nsTArray<int16_t >& a, nsTArray<int16_t >& b, nsTArray<int16_t >& _retval) = 0;

  /* Array<double> testDoubleSequence (in Array<double> a, inout Array<double> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestDoubleSequence(const nsTArray<double >& a, nsTArray<double >& b, nsTArray<double >& _retval) = 0;

  /* Array<nsIXPCTestInterfaceA> testInterfaceSequence (in Array<nsIXPCTestInterfaceA> a, inout Array<nsIXPCTestInterfaceA> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestInterfaceSequence(const nsTArray<RefPtr<nsIXPCTestInterfaceA>>& a, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& b, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& _retval) = 0;

  /* Array<AString> testAStringSequence (in Array<AString> a, inout Array<AString> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestAStringSequence(const nsTArray<nsString >& a, nsTArray<nsString >& b, nsTArray<nsString >& _retval) = 0;

  /* Array<ACString> testACStringSequence (in Array<ACString> a, inout Array<ACString> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestACStringSequence(const nsTArray<nsCString >& a, nsTArray<nsCString >& b, nsTArray<nsCString >& _retval) = 0;

  /* Array<jsval> testJsvalSequence (in Array<jsval> a, inout Array<jsval> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestJsvalSequence(const nsTArray<JS::Value >& a, nsTArray<JS::Value >& b, nsTArray<JS::Value >& _retval) = 0;

  /* Array<Array<short>> testSequenceSequence (in Array<Array<short>> a, inout Array<Array<short>> b); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestSequenceSequence(const nsTArray<nsTArray<int16_t >>& a, nsTArray<nsTArray<int16_t >>& b, nsTArray<nsTArray<int16_t >>& _retval) = 0;

  /* void testInterfaceIsSequence (in nsIIDPtr aIID, [iid_is (aIID)] in Array<nsQIResult> a, inout nsIIDPtr bIID, [iid_is (bIID)] inout Array<nsQIResult> b, out nsIIDPtr rvIID, [iid_is (rvIID), retval] out Array<nsQIResult> rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestInterfaceIsSequence(const nsIID * aIID, const nsTArray<void * >& a, nsIID * * bIID, nsTArray<void * >& b, nsIID * * rvIID, nsTArray<void * >& rv) = 0;

  /* Array<uint8_t> testOptionalSequence ([optional] in Array<uint8_t> arr); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestOptionalSequence(const nsTArray<uint8_t >& arr, nsTArray<uint8_t >& _retval) = 0;

  /* void testShortArray (in unsigned long aLength, [array, size_is (aLength)] in short a, inout unsigned long bLength, [array, size_is (bLength)] inout short b, out unsigned long rvLength, [array, size_is (rvLength), retval] out short rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestShortArray(uint32_t aLength, int16_t *a, uint32_t *bLength, int16_t **b, uint32_t *rvLength, int16_t **rv) = 0;

  /* void testDoubleArray (in unsigned long aLength, [array, size_is (aLength)] in double a, inout unsigned long bLength, [array, size_is (bLength)] inout double b, out unsigned long rvLength, [array, size_is (rvLength), retval] out double rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestDoubleArray(uint32_t aLength, double *a, uint32_t *bLength, double **b, uint32_t *rvLength, double **rv) = 0;

  /* void testStringArray (in unsigned long aLength, [array, size_is (aLength)] in string a, inout unsigned long bLength, [array, size_is (bLength)] inout string b, out unsigned long rvLength, [array, size_is (rvLength), retval] out string rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestStringArray(uint32_t aLength, const char * *a, uint32_t *bLength, char * **b, uint32_t *rvLength, char * **rv) = 0;

  /* void testWstringArray (in unsigned long aLength, [array, size_is (aLength)] in wstring a, inout unsigned long bLength, [array, size_is (bLength)] inout wstring b, out unsigned long rvLength, [array, size_is (rvLength), retval] out wstring rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestWstringArray(uint32_t aLength, const char16_t * *a, uint32_t *bLength, char16_t * **b, uint32_t *rvLength, char16_t * **rv) = 0;

  /* void testInterfaceArray (in unsigned long aLength, [array, size_is (aLength)] in nsIXPCTestInterfaceA a, inout unsigned long bLength, [array, size_is (bLength)] inout nsIXPCTestInterfaceA b, out unsigned long rvLength, [array, size_is (rvLength), retval] out nsIXPCTestInterfaceA rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestInterfaceArray(uint32_t aLength, nsIXPCTestInterfaceA **a, uint32_t *bLength, nsIXPCTestInterfaceA ***b, uint32_t *rvLength, nsIXPCTestInterfaceA ***rv) = 0;

  /* unsigned long testByteArrayOptionalLength ([array, size_is (aLength)] in uint8_t a, [optional] in unsigned long aLength); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestByteArrayOptionalLength(uint8_t *a, uint32_t aLength, uint32_t *_retval) = 0;

  /* void testSizedString (in unsigned long aLength, [size_is (aLength)] in string a, inout unsigned long bLength, [size_is (bLength)] inout string b, out unsigned long rvLength, [size_is (rvLength), retval] out string rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestSizedString(uint32_t aLength, const char * a, uint32_t *bLength, char * *b, uint32_t *rvLength, char * *rv) = 0;

  /* void testSizedWstring (in unsigned long aLength, [size_is (aLength)] in wstring a, inout unsigned long bLength, [size_is (bLength)] inout wstring b, out unsigned long rvLength, [size_is (rvLength), retval] out wstring rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestSizedWstring(uint32_t aLength, const char16_t * a, uint32_t *bLength, char16_t * *b, uint32_t *rvLength, char16_t * *rv) = 0;

  /* void testInterfaceIs (in nsIIDPtr aIID, [iid_is (aIID)] in nsQIResult a, inout nsIIDPtr bIID, [iid_is (bIID)] inout nsQIResult b, out nsIIDPtr rvIID, [iid_is (rvIID), retval] out nsQIResult rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestInterfaceIs(const nsIID * aIID, void * a, nsIID * * bIID, void * * b, nsIID * * rvIID, void * * rv) = 0;

  /* void testInterfaceIsArray (in unsigned long aLength, in nsIIDPtr aIID, [array, size_is (aLength), iid_is (aIID)] in nsQIResult a, inout unsigned long bLength, inout nsIIDPtr bIID, [array, size_is (bLength), iid_is (bIID)] inout nsQIResult b, out unsigned long rvLength, out nsIIDPtr rvIID, [retval, array, size_is (rvLength), iid_is (rvIID)] out nsQIResult rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestInterfaceIsArray(uint32_t aLength, const nsIID * aIID, void * *a, uint32_t *bLength, nsIID * * bIID, void * **b, uint32_t *rvLength, nsIID * * rvIID, void * **rv) = 0;

  /* void testJsvalArray (in unsigned long aLength, [array, size_is (aLength)] in jsval a, inout unsigned long bLength, [array, size_is (bLength)] inout jsval b, out unsigned long rvLength, [array, size_is (rvLength), retval] out jsval rv); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestJsvalArray(uint32_t aLength, JS::Value *a, uint32_t *bLength, JS::Value **b, uint32_t *rvLength, JS::Value **rv) = 0;

  /* void testOutAString (out AString o); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestOutAString(nsAString& o) = 0;

  /* ACString testStringArrayOptionalSize ([array, size_is (aLength)] in string a, [optional] in unsigned long aLength); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestStringArrayOptionalSize(const char * *a, uint32_t aLength, nsACString& _retval) = 0;

  /* void testOmittedOptionalOut ([optional] out nsIURI aOut); */
  JS_HAZ_CAN_RUN_SCRIPT NS_IMETHOD TestOmittedOptionalOut(nsIURI **aOut = nullptr) = 0;

};

  NS_DEFINE_STATIC_IID_ACCESSOR(nsIXPCTestParams, NS_IXPCTESTPARAMS_IID)

/* Use this macro when declaring classes that implement this interface. */
#define NS_DECL_NSIXPCTESTPARAMS \
  NS_IMETHOD TestBoolean(bool a, bool *b, bool *_retval) override; \
  NS_IMETHOD TestOctet(uint8_t a, uint8_t *b, uint8_t *_retval) override; \
  NS_IMETHOD TestShort(int16_t a, int16_t *b, int16_t *_retval) override; \
  NS_IMETHOD TestLong(int32_t a, int32_t *b, int32_t *_retval) override; \
  NS_IMETHOD TestLongLong(int64_t a, int64_t *b, int64_t *_retval) override; \
  NS_IMETHOD TestUnsignedShort(uint16_t a, uint16_t *b, uint16_t *_retval) override; \
  NS_IMETHOD TestUnsignedLong(uint32_t a, uint32_t *b, uint32_t *_retval) override; \
  NS_IMETHOD TestUnsignedLongLong(uint64_t a, uint64_t *b, uint64_t *_retval) override; \
  NS_IMETHOD TestFloat(float a, float *b, float *_retval) override; \
  NS_IMETHOD TestDouble(double a, float *b, double *_retval) override; \
  NS_IMETHOD TestChar(char a, char *b, char *_retval) override; \
  NS_IMETHOD TestString(const char * a, char * *b, char * *_retval) override; \
  NS_IMETHOD TestWchar(char16_t a, char16_t *b, char16_t *_retval) override; \
  NS_IMETHOD TestWstring(const char16_t * a, char16_t * *b, char16_t * *_retval) override; \
  NS_IMETHOD TestAString(const nsAString& a, nsAString& b, nsAString& _retval) override; \
  NS_IMETHOD TestAUTF8String(const nsACString& a, nsACString& b, nsACString& _retval) override; \
  NS_IMETHOD TestACString(const nsACString& a, nsACString& b, nsACString& _retval) override; \
  NS_IMETHOD TestJsval(JS::HandleValue a, JS::MutableHandleValue b, JS::MutableHandleValue _retval) override; \
  NS_IMETHOD TestShortSequence(const nsTArray<int16_t >& a, nsTArray<int16_t >& b, nsTArray<int16_t >& _retval) override; \
  NS_IMETHOD TestDoubleSequence(const nsTArray<double >& a, nsTArray<double >& b, nsTArray<double >& _retval) override; \
  NS_IMETHOD TestInterfaceSequence(const nsTArray<RefPtr<nsIXPCTestInterfaceA>>& a, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& b, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& _retval) override; \
  NS_IMETHOD TestAStringSequence(const nsTArray<nsString >& a, nsTArray<nsString >& b, nsTArray<nsString >& _retval) override; \
  NS_IMETHOD TestACStringSequence(const nsTArray<nsCString >& a, nsTArray<nsCString >& b, nsTArray<nsCString >& _retval) override; \
  NS_IMETHOD TestJsvalSequence(const nsTArray<JS::Value >& a, nsTArray<JS::Value >& b, nsTArray<JS::Value >& _retval) override; \
  NS_IMETHOD TestSequenceSequence(const nsTArray<nsTArray<int16_t >>& a, nsTArray<nsTArray<int16_t >>& b, nsTArray<nsTArray<int16_t >>& _retval) override; \
  NS_IMETHOD TestInterfaceIsSequence(const nsIID * aIID, const nsTArray<void * >& a, nsIID * * bIID, nsTArray<void * >& b, nsIID * * rvIID, nsTArray<void * >& rv) override; \
  NS_IMETHOD TestOptionalSequence(const nsTArray<uint8_t >& arr, nsTArray<uint8_t >& _retval) override; \
  NS_IMETHOD TestShortArray(uint32_t aLength, int16_t *a, uint32_t *bLength, int16_t **b, uint32_t *rvLength, int16_t **rv) override; \
  NS_IMETHOD TestDoubleArray(uint32_t aLength, double *a, uint32_t *bLength, double **b, uint32_t *rvLength, double **rv) override; \
  NS_IMETHOD TestStringArray(uint32_t aLength, const char * *a, uint32_t *bLength, char * **b, uint32_t *rvLength, char * **rv) override; \
  NS_IMETHOD TestWstringArray(uint32_t aLength, const char16_t * *a, uint32_t *bLength, char16_t * **b, uint32_t *rvLength, char16_t * **rv) override; \
  NS_IMETHOD TestInterfaceArray(uint32_t aLength, nsIXPCTestInterfaceA **a, uint32_t *bLength, nsIXPCTestInterfaceA ***b, uint32_t *rvLength, nsIXPCTestInterfaceA ***rv) override; \
  NS_IMETHOD TestByteArrayOptionalLength(uint8_t *a, uint32_t aLength, uint32_t *_retval) override; \
  NS_IMETHOD TestSizedString(uint32_t aLength, const char * a, uint32_t *bLength, char * *b, uint32_t *rvLength, char * *rv) override; \
  NS_IMETHOD TestSizedWstring(uint32_t aLength, const char16_t * a, uint32_t *bLength, char16_t * *b, uint32_t *rvLength, char16_t * *rv) override; \
  NS_IMETHOD TestInterfaceIs(const nsIID * aIID, void * a, nsIID * * bIID, void * * b, nsIID * * rvIID, void * * rv) override; \
  NS_IMETHOD TestInterfaceIsArray(uint32_t aLength, const nsIID * aIID, void * *a, uint32_t *bLength, nsIID * * bIID, void * **b, uint32_t *rvLength, nsIID * * rvIID, void * **rv) override; \
  NS_IMETHOD TestJsvalArray(uint32_t aLength, JS::Value *a, uint32_t *bLength, JS::Value **b, uint32_t *rvLength, JS::Value **rv) override; \
  NS_IMETHOD TestOutAString(nsAString& o) override; \
  NS_IMETHOD TestStringArrayOptionalSize(const char * *a, uint32_t aLength, nsACString& _retval) override; \
  NS_IMETHOD TestOmittedOptionalOut(nsIURI **aOut = nullptr) override; 

/* Use this macro when declaring the members of this interface when the
   class doesn't implement the interface. This is useful for forwarding. */
#define NS_DECL_NON_VIRTUAL_NSIXPCTESTPARAMS \
  nsresult TestBoolean(bool a, bool *b, bool *_retval); \
  nsresult TestOctet(uint8_t a, uint8_t *b, uint8_t *_retval); \
  nsresult TestShort(int16_t a, int16_t *b, int16_t *_retval); \
  nsresult TestLong(int32_t a, int32_t *b, int32_t *_retval); \
  nsresult TestLongLong(int64_t a, int64_t *b, int64_t *_retval); \
  nsresult TestUnsignedShort(uint16_t a, uint16_t *b, uint16_t *_retval); \
  nsresult TestUnsignedLong(uint32_t a, uint32_t *b, uint32_t *_retval); \
  nsresult TestUnsignedLongLong(uint64_t a, uint64_t *b, uint64_t *_retval); \
  nsresult TestFloat(float a, float *b, float *_retval); \
  nsresult TestDouble(double a, float *b, double *_retval); \
  nsresult TestChar(char a, char *b, char *_retval); \
  nsresult TestString(const char * a, char * *b, char * *_retval); \
  nsresult TestWchar(char16_t a, char16_t *b, char16_t *_retval); \
  nsresult TestWstring(const char16_t * a, char16_t * *b, char16_t * *_retval); \
  nsresult TestAString(const nsAString& a, nsAString& b, nsAString& _retval); \
  nsresult TestAUTF8String(const nsACString& a, nsACString& b, nsACString& _retval); \
  nsresult TestACString(const nsACString& a, nsACString& b, nsACString& _retval); \
  nsresult TestJsval(JS::HandleValue a, JS::MutableHandleValue b, JS::MutableHandleValue _retval); \
  nsresult TestShortSequence(const nsTArray<int16_t >& a, nsTArray<int16_t >& b, nsTArray<int16_t >& _retval); \
  nsresult TestDoubleSequence(const nsTArray<double >& a, nsTArray<double >& b, nsTArray<double >& _retval); \
  nsresult TestInterfaceSequence(const nsTArray<RefPtr<nsIXPCTestInterfaceA>>& a, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& b, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& _retval); \
  nsresult TestAStringSequence(const nsTArray<nsString >& a, nsTArray<nsString >& b, nsTArray<nsString >& _retval); \
  nsresult TestACStringSequence(const nsTArray<nsCString >& a, nsTArray<nsCString >& b, nsTArray<nsCString >& _retval); \
  nsresult TestJsvalSequence(const nsTArray<JS::Value >& a, nsTArray<JS::Value >& b, nsTArray<JS::Value >& _retval); \
  nsresult TestSequenceSequence(const nsTArray<nsTArray<int16_t >>& a, nsTArray<nsTArray<int16_t >>& b, nsTArray<nsTArray<int16_t >>& _retval); \
  nsresult TestInterfaceIsSequence(const nsIID * aIID, const nsTArray<void * >& a, nsIID * * bIID, nsTArray<void * >& b, nsIID * * rvIID, nsTArray<void * >& rv); \
  nsresult TestOptionalSequence(const nsTArray<uint8_t >& arr, nsTArray<uint8_t >& _retval); \
  nsresult TestShortArray(uint32_t aLength, int16_t *a, uint32_t *bLength, int16_t **b, uint32_t *rvLength, int16_t **rv); \
  nsresult TestDoubleArray(uint32_t aLength, double *a, uint32_t *bLength, double **b, uint32_t *rvLength, double **rv); \
  nsresult TestStringArray(uint32_t aLength, const char * *a, uint32_t *bLength, char * **b, uint32_t *rvLength, char * **rv); \
  nsresult TestWstringArray(uint32_t aLength, const char16_t * *a, uint32_t *bLength, char16_t * **b, uint32_t *rvLength, char16_t * **rv); \
  nsresult TestInterfaceArray(uint32_t aLength, nsIXPCTestInterfaceA **a, uint32_t *bLength, nsIXPCTestInterfaceA ***b, uint32_t *rvLength, nsIXPCTestInterfaceA ***rv); \
  nsresult TestByteArrayOptionalLength(uint8_t *a, uint32_t aLength, uint32_t *_retval); \
  nsresult TestSizedString(uint32_t aLength, const char * a, uint32_t *bLength, char * *b, uint32_t *rvLength, char * *rv); \
  nsresult TestSizedWstring(uint32_t aLength, const char16_t * a, uint32_t *bLength, char16_t * *b, uint32_t *rvLength, char16_t * *rv); \
  nsresult TestInterfaceIs(const nsIID * aIID, void * a, nsIID * * bIID, void * * b, nsIID * * rvIID, void * * rv); \
  nsresult TestInterfaceIsArray(uint32_t aLength, const nsIID * aIID, void * *a, uint32_t *bLength, nsIID * * bIID, void * **b, uint32_t *rvLength, nsIID * * rvIID, void * **rv); \
  nsresult TestJsvalArray(uint32_t aLength, JS::Value *a, uint32_t *bLength, JS::Value **b, uint32_t *rvLength, JS::Value **rv); \
  nsresult TestOutAString(nsAString& o); \
  nsresult TestStringArrayOptionalSize(const char * *a, uint32_t aLength, nsACString& _retval); \
  nsresult TestOmittedOptionalOut(nsIURI **aOut = nullptr); 

/* Use this macro to declare functions that forward the behavior of this interface to another object. */
#define NS_FORWARD_NSIXPCTESTPARAMS(_to) \
  NS_IMETHOD TestBoolean(bool a, bool *b, bool *_retval) override { return _to TestBoolean(a, b, _retval); } \
  NS_IMETHOD TestOctet(uint8_t a, uint8_t *b, uint8_t *_retval) override { return _to TestOctet(a, b, _retval); } \
  NS_IMETHOD TestShort(int16_t a, int16_t *b, int16_t *_retval) override { return _to TestShort(a, b, _retval); } \
  NS_IMETHOD TestLong(int32_t a, int32_t *b, int32_t *_retval) override { return _to TestLong(a, b, _retval); } \
  NS_IMETHOD TestLongLong(int64_t a, int64_t *b, int64_t *_retval) override { return _to TestLongLong(a, b, _retval); } \
  NS_IMETHOD TestUnsignedShort(uint16_t a, uint16_t *b, uint16_t *_retval) override { return _to TestUnsignedShort(a, b, _retval); } \
  NS_IMETHOD TestUnsignedLong(uint32_t a, uint32_t *b, uint32_t *_retval) override { return _to TestUnsignedLong(a, b, _retval); } \
  NS_IMETHOD TestUnsignedLongLong(uint64_t a, uint64_t *b, uint64_t *_retval) override { return _to TestUnsignedLongLong(a, b, _retval); } \
  NS_IMETHOD TestFloat(float a, float *b, float *_retval) override { return _to TestFloat(a, b, _retval); } \
  NS_IMETHOD TestDouble(double a, float *b, double *_retval) override { return _to TestDouble(a, b, _retval); } \
  NS_IMETHOD TestChar(char a, char *b, char *_retval) override { return _to TestChar(a, b, _retval); } \
  NS_IMETHOD TestString(const char * a, char * *b, char * *_retval) override { return _to TestString(a, b, _retval); } \
  NS_IMETHOD TestWchar(char16_t a, char16_t *b, char16_t *_retval) override { return _to TestWchar(a, b, _retval); } \
  NS_IMETHOD TestWstring(const char16_t * a, char16_t * *b, char16_t * *_retval) override { return _to TestWstring(a, b, _retval); } \
  NS_IMETHOD TestAString(const nsAString& a, nsAString& b, nsAString& _retval) override { return _to TestAString(a, b, _retval); } \
  NS_IMETHOD TestAUTF8String(const nsACString& a, nsACString& b, nsACString& _retval) override { return _to TestAUTF8String(a, b, _retval); } \
  NS_IMETHOD TestACString(const nsACString& a, nsACString& b, nsACString& _retval) override { return _to TestACString(a, b, _retval); } \
  NS_IMETHOD TestJsval(JS::HandleValue a, JS::MutableHandleValue b, JS::MutableHandleValue _retval) override { return _to TestJsval(a, b, _retval); } \
  NS_IMETHOD TestShortSequence(const nsTArray<int16_t >& a, nsTArray<int16_t >& b, nsTArray<int16_t >& _retval) override { return _to TestShortSequence(a, b, _retval); } \
  NS_IMETHOD TestDoubleSequence(const nsTArray<double >& a, nsTArray<double >& b, nsTArray<double >& _retval) override { return _to TestDoubleSequence(a, b, _retval); } \
  NS_IMETHOD TestInterfaceSequence(const nsTArray<RefPtr<nsIXPCTestInterfaceA>>& a, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& b, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& _retval) override { return _to TestInterfaceSequence(a, b, _retval); } \
  NS_IMETHOD TestAStringSequence(const nsTArray<nsString >& a, nsTArray<nsString >& b, nsTArray<nsString >& _retval) override { return _to TestAStringSequence(a, b, _retval); } \
  NS_IMETHOD TestACStringSequence(const nsTArray<nsCString >& a, nsTArray<nsCString >& b, nsTArray<nsCString >& _retval) override { return _to TestACStringSequence(a, b, _retval); } \
  NS_IMETHOD TestJsvalSequence(const nsTArray<JS::Value >& a, nsTArray<JS::Value >& b, nsTArray<JS::Value >& _retval) override { return _to TestJsvalSequence(a, b, _retval); } \
  NS_IMETHOD TestSequenceSequence(const nsTArray<nsTArray<int16_t >>& a, nsTArray<nsTArray<int16_t >>& b, nsTArray<nsTArray<int16_t >>& _retval) override { return _to TestSequenceSequence(a, b, _retval); } \
  NS_IMETHOD TestInterfaceIsSequence(const nsIID * aIID, const nsTArray<void * >& a, nsIID * * bIID, nsTArray<void * >& b, nsIID * * rvIID, nsTArray<void * >& rv) override { return _to TestInterfaceIsSequence(aIID, a, bIID, b, rvIID, rv); } \
  NS_IMETHOD TestOptionalSequence(const nsTArray<uint8_t >& arr, nsTArray<uint8_t >& _retval) override { return _to TestOptionalSequence(arr, _retval); } \
  NS_IMETHOD TestShortArray(uint32_t aLength, int16_t *a, uint32_t *bLength, int16_t **b, uint32_t *rvLength, int16_t **rv) override { return _to TestShortArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestDoubleArray(uint32_t aLength, double *a, uint32_t *bLength, double **b, uint32_t *rvLength, double **rv) override { return _to TestDoubleArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestStringArray(uint32_t aLength, const char * *a, uint32_t *bLength, char * **b, uint32_t *rvLength, char * **rv) override { return _to TestStringArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestWstringArray(uint32_t aLength, const char16_t * *a, uint32_t *bLength, char16_t * **b, uint32_t *rvLength, char16_t * **rv) override { return _to TestWstringArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestInterfaceArray(uint32_t aLength, nsIXPCTestInterfaceA **a, uint32_t *bLength, nsIXPCTestInterfaceA ***b, uint32_t *rvLength, nsIXPCTestInterfaceA ***rv) override { return _to TestInterfaceArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestByteArrayOptionalLength(uint8_t *a, uint32_t aLength, uint32_t *_retval) override { return _to TestByteArrayOptionalLength(a, aLength, _retval); } \
  NS_IMETHOD TestSizedString(uint32_t aLength, const char * a, uint32_t *bLength, char * *b, uint32_t *rvLength, char * *rv) override { return _to TestSizedString(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestSizedWstring(uint32_t aLength, const char16_t * a, uint32_t *bLength, char16_t * *b, uint32_t *rvLength, char16_t * *rv) override { return _to TestSizedWstring(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestInterfaceIs(const nsIID * aIID, void * a, nsIID * * bIID, void * * b, nsIID * * rvIID, void * * rv) override { return _to TestInterfaceIs(aIID, a, bIID, b, rvIID, rv); } \
  NS_IMETHOD TestInterfaceIsArray(uint32_t aLength, const nsIID * aIID, void * *a, uint32_t *bLength, nsIID * * bIID, void * **b, uint32_t *rvLength, nsIID * * rvIID, void * **rv) override { return _to TestInterfaceIsArray(aLength, aIID, a, bLength, bIID, b, rvLength, rvIID, rv); } \
  NS_IMETHOD TestJsvalArray(uint32_t aLength, JS::Value *a, uint32_t *bLength, JS::Value **b, uint32_t *rvLength, JS::Value **rv) override { return _to TestJsvalArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestOutAString(nsAString& o) override { return _to TestOutAString(o); } \
  NS_IMETHOD TestStringArrayOptionalSize(const char * *a, uint32_t aLength, nsACString& _retval) override { return _to TestStringArrayOptionalSize(a, aLength, _retval); } \
  NS_IMETHOD TestOmittedOptionalOut(nsIURI **aOut = nullptr) override { return _to TestOmittedOptionalOut(aOut); } 

/* Use this macro to declare functions that forward the behavior of this interface to another object in a safe way. */
#define NS_FORWARD_SAFE_NSIXPCTESTPARAMS(_to) \
  NS_IMETHOD TestBoolean(bool a, bool *b, bool *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestBoolean(a, b, _retval); } \
  NS_IMETHOD TestOctet(uint8_t a, uint8_t *b, uint8_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestOctet(a, b, _retval); } \
  NS_IMETHOD TestShort(int16_t a, int16_t *b, int16_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestShort(a, b, _retval); } \
  NS_IMETHOD TestLong(int32_t a, int32_t *b, int32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestLong(a, b, _retval); } \
  NS_IMETHOD TestLongLong(int64_t a, int64_t *b, int64_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestLongLong(a, b, _retval); } \
  NS_IMETHOD TestUnsignedShort(uint16_t a, uint16_t *b, uint16_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestUnsignedShort(a, b, _retval); } \
  NS_IMETHOD TestUnsignedLong(uint32_t a, uint32_t *b, uint32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestUnsignedLong(a, b, _retval); } \
  NS_IMETHOD TestUnsignedLongLong(uint64_t a, uint64_t *b, uint64_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestUnsignedLongLong(a, b, _retval); } \
  NS_IMETHOD TestFloat(float a, float *b, float *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestFloat(a, b, _retval); } \
  NS_IMETHOD TestDouble(double a, float *b, double *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestDouble(a, b, _retval); } \
  NS_IMETHOD TestChar(char a, char *b, char *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestChar(a, b, _retval); } \
  NS_IMETHOD TestString(const char * a, char * *b, char * *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestString(a, b, _retval); } \
  NS_IMETHOD TestWchar(char16_t a, char16_t *b, char16_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestWchar(a, b, _retval); } \
  NS_IMETHOD TestWstring(const char16_t * a, char16_t * *b, char16_t * *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestWstring(a, b, _retval); } \
  NS_IMETHOD TestAString(const nsAString& a, nsAString& b, nsAString& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestAString(a, b, _retval); } \
  NS_IMETHOD TestAUTF8String(const nsACString& a, nsACString& b, nsACString& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestAUTF8String(a, b, _retval); } \
  NS_IMETHOD TestACString(const nsACString& a, nsACString& b, nsACString& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestACString(a, b, _retval); } \
  NS_IMETHOD TestJsval(JS::HandleValue a, JS::MutableHandleValue b, JS::MutableHandleValue _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestJsval(a, b, _retval); } \
  NS_IMETHOD TestShortSequence(const nsTArray<int16_t >& a, nsTArray<int16_t >& b, nsTArray<int16_t >& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestShortSequence(a, b, _retval); } \
  NS_IMETHOD TestDoubleSequence(const nsTArray<double >& a, nsTArray<double >& b, nsTArray<double >& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestDoubleSequence(a, b, _retval); } \
  NS_IMETHOD TestInterfaceSequence(const nsTArray<RefPtr<nsIXPCTestInterfaceA>>& a, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& b, nsTArray<RefPtr<nsIXPCTestInterfaceA>>& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestInterfaceSequence(a, b, _retval); } \
  NS_IMETHOD TestAStringSequence(const nsTArray<nsString >& a, nsTArray<nsString >& b, nsTArray<nsString >& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestAStringSequence(a, b, _retval); } \
  NS_IMETHOD TestACStringSequence(const nsTArray<nsCString >& a, nsTArray<nsCString >& b, nsTArray<nsCString >& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestACStringSequence(a, b, _retval); } \
  NS_IMETHOD TestJsvalSequence(const nsTArray<JS::Value >& a, nsTArray<JS::Value >& b, nsTArray<JS::Value >& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestJsvalSequence(a, b, _retval); } \
  NS_IMETHOD TestSequenceSequence(const nsTArray<nsTArray<int16_t >>& a, nsTArray<nsTArray<int16_t >>& b, nsTArray<nsTArray<int16_t >>& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestSequenceSequence(a, b, _retval); } \
  NS_IMETHOD TestInterfaceIsSequence(const nsIID * aIID, const nsTArray<void * >& a, nsIID * * bIID, nsTArray<void * >& b, nsIID * * rvIID, nsTArray<void * >& rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestInterfaceIsSequence(aIID, a, bIID, b, rvIID, rv); } \
  NS_IMETHOD TestOptionalSequence(const nsTArray<uint8_t >& arr, nsTArray<uint8_t >& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestOptionalSequence(arr, _retval); } \
  NS_IMETHOD TestShortArray(uint32_t aLength, int16_t *a, uint32_t *bLength, int16_t **b, uint32_t *rvLength, int16_t **rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestShortArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestDoubleArray(uint32_t aLength, double *a, uint32_t *bLength, double **b, uint32_t *rvLength, double **rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestDoubleArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestStringArray(uint32_t aLength, const char * *a, uint32_t *bLength, char * **b, uint32_t *rvLength, char * **rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestStringArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestWstringArray(uint32_t aLength, const char16_t * *a, uint32_t *bLength, char16_t * **b, uint32_t *rvLength, char16_t * **rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestWstringArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestInterfaceArray(uint32_t aLength, nsIXPCTestInterfaceA **a, uint32_t *bLength, nsIXPCTestInterfaceA ***b, uint32_t *rvLength, nsIXPCTestInterfaceA ***rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestInterfaceArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestByteArrayOptionalLength(uint8_t *a, uint32_t aLength, uint32_t *_retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestByteArrayOptionalLength(a, aLength, _retval); } \
  NS_IMETHOD TestSizedString(uint32_t aLength, const char * a, uint32_t *bLength, char * *b, uint32_t *rvLength, char * *rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestSizedString(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestSizedWstring(uint32_t aLength, const char16_t * a, uint32_t *bLength, char16_t * *b, uint32_t *rvLength, char16_t * *rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestSizedWstring(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestInterfaceIs(const nsIID * aIID, void * a, nsIID * * bIID, void * * b, nsIID * * rvIID, void * * rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestInterfaceIs(aIID, a, bIID, b, rvIID, rv); } \
  NS_IMETHOD TestInterfaceIsArray(uint32_t aLength, const nsIID * aIID, void * *a, uint32_t *bLength, nsIID * * bIID, void * **b, uint32_t *rvLength, nsIID * * rvIID, void * **rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestInterfaceIsArray(aLength, aIID, a, bLength, bIID, b, rvLength, rvIID, rv); } \
  NS_IMETHOD TestJsvalArray(uint32_t aLength, JS::Value *a, uint32_t *bLength, JS::Value **b, uint32_t *rvLength, JS::Value **rv) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestJsvalArray(aLength, a, bLength, b, rvLength, rv); } \
  NS_IMETHOD TestOutAString(nsAString& o) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestOutAString(o); } \
  NS_IMETHOD TestStringArrayOptionalSize(const char * *a, uint32_t aLength, nsACString& _retval) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestStringArrayOptionalSize(a, aLength, _retval); } \
  NS_IMETHOD TestOmittedOptionalOut(nsIURI **aOut = nullptr) override { return !_to ? NS_ERROR_NULL_POINTER : _to->TestOmittedOptionalOut(aOut); } 


#endif /* __gen_xpctest_params_h__ */
