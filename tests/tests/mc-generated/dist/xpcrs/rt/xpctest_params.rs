//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_params.idl
//


/// `interface nsIXPCTestParams : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestParams {
    vtable: *const nsIXPCTestParamsVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestParams.
unsafe impl XpCom for nsIXPCTestParams {
    const IID: nsIID = nsID(0x812145c7, 0x9fcc, 0x425e,
        [0xa8, 0x78, 0x36, 0xad, 0x1b, 0x77, 0x30, 0xb7]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestParams {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestParams.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestParamsCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestParams`.
    fn coerce_from(v: &nsIXPCTestParams) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestParamsCoerce for nsIXPCTestParams {
    #[inline]
    fn coerce_from(v: &nsIXPCTestParams) -> &Self {
        v
    }
}

impl nsIXPCTestParams {
    /// Cast this `nsIXPCTestParams` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestParamsCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestParams {
    type Target = nsISupports;
    #[inline]
    fn deref(&self) -> &nsISupports {
        unsafe {
            ::std::mem::transmute(self)
        }
    }
}

// Ensure we can use .coerce() to cast to our base types as well. Any type which
// our base interface can coerce from should be coercable from us as well.
impl<T: nsISupportsCoerce> nsIXPCTestParamsCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestParams) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestParams
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestParamsVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* boolean testBoolean (in boolean a, inout boolean b); */
    pub TestBoolean: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: bool, b: *mut bool, _retval: *mut bool) -> ::nserror::nsresult,

    /* octet testOctet (in octet a, inout octet b); */
    pub TestOctet: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: u8, b: *mut u8, _retval: *mut u8) -> ::nserror::nsresult,

    /* short testShort (in short a, inout short b); */
    pub TestShort: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: i16, b: *mut i16, _retval: *mut i16) -> ::nserror::nsresult,

    /* long testLong (in long a, inout long b); */
    pub TestLong: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: i32, b: *mut i32, _retval: *mut i32) -> ::nserror::nsresult,

    /* long long testLongLong (in long long a, inout long long b); */
    pub TestLongLong: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: i64, b: *mut i64, _retval: *mut i64) -> ::nserror::nsresult,

    /* unsigned short testUnsignedShort (in unsigned short a, inout unsigned short b); */
    pub TestUnsignedShort: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: u16, b: *mut u16, _retval: *mut u16) -> ::nserror::nsresult,

    /* unsigned long testUnsignedLong (in unsigned long a, inout unsigned long b); */
    pub TestUnsignedLong: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: u32, b: *mut u32, _retval: *mut u32) -> ::nserror::nsresult,

    /* unsigned long long testUnsignedLongLong (in unsigned long long a, inout unsigned long long b); */
    pub TestUnsignedLongLong: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: u64, b: *mut u64, _retval: *mut u64) -> ::nserror::nsresult,

    /* float testFloat (in float a, inout float b); */
    pub TestFloat: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: libc::c_float, b: *mut libc::c_float, _retval: *mut libc::c_float) -> ::nserror::nsresult,

    /* double testDouble (in double a, inout float b); */
    pub TestDouble: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: libc::c_double, b: *mut libc::c_float, _retval: *mut libc::c_double) -> ::nserror::nsresult,

    /* char testChar (in char a, inout char b); */
    pub TestChar: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: libc::c_char, b: *mut libc::c_char, _retval: *mut libc::c_char) -> ::nserror::nsresult,

    /* string testString (in string a, inout string b); */
    pub TestString: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const libc::c_char, b: *mut *const libc::c_char, _retval: *mut *const libc::c_char) -> ::nserror::nsresult,

    /* wchar testWchar (in wchar a, inout wchar b); */
    pub TestWchar: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: i16, b: *mut i16, _retval: *mut i16) -> ::nserror::nsresult,

    /* wstring testWstring (in wstring a, inout wstring b); */
    pub TestWstring: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const i16, b: *mut *const i16, _retval: *mut *const i16) -> ::nserror::nsresult,

    /* AString testAString (in AString a, inout AString b); */
    pub TestAString: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const ::nsstring::nsAString, b: *mut ::nsstring::nsAString, _retval: *mut ::nsstring::nsAString) -> ::nserror::nsresult,

    /* AUTF8String testAUTF8String (in AUTF8String a, inout AUTF8String b); */
    pub TestAUTF8String: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const ::nsstring::nsACString, b: *mut ::nsstring::nsACString, _retval: *mut ::nsstring::nsACString) -> ::nserror::nsresult,

    /* ACString testACString (in ACString a, inout ACString b); */
    pub TestACString: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const ::nsstring::nsACString, b: *mut ::nsstring::nsACString, _retval: *mut ::nsstring::nsACString) -> ::nserror::nsresult,

    /* jsval testJsval (in jsval a, inout jsval b); */
    /// Unable to generate binding because `specialtype jsval unsupported`
    pub TestJsval: *const ::libc::c_void,

    /* Array<short> testShortSequence (in Array<short> a, inout Array<short> b); */
    pub TestShortSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const thin_vec::ThinVec<i16>, b: *mut thin_vec::ThinVec<i16>, _retval: *mut thin_vec::ThinVec<i16>) -> ::nserror::nsresult,

    /* Array<double> testDoubleSequence (in Array<double> a, inout Array<double> b); */
    pub TestDoubleSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const thin_vec::ThinVec<libc::c_double>, b: *mut thin_vec::ThinVec<libc::c_double>, _retval: *mut thin_vec::ThinVec<libc::c_double>) -> ::nserror::nsresult,

    /* Array<nsIXPCTestInterfaceA> testInterfaceSequence (in Array<nsIXPCTestInterfaceA> a, inout Array<nsIXPCTestInterfaceA> b); */
    pub TestInterfaceSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const thin_vec::ThinVec<RefPtr<nsIXPCTestInterfaceA>>, b: *mut thin_vec::ThinVec<RefPtr<nsIXPCTestInterfaceA>>, _retval: *mut thin_vec::ThinVec<RefPtr<nsIXPCTestInterfaceA>>) -> ::nserror::nsresult,

    /* Array<AString> testAStringSequence (in Array<AString> a, inout Array<AString> b); */
    pub TestAStringSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const thin_vec::ThinVec<::nsstring::nsString>, b: *mut thin_vec::ThinVec<::nsstring::nsString>, _retval: *mut thin_vec::ThinVec<::nsstring::nsString>) -> ::nserror::nsresult,

    /* Array<ACString> testACStringSequence (in Array<ACString> a, inout Array<ACString> b); */
    pub TestACStringSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const thin_vec::ThinVec<::nsstring::nsCString>, b: *mut thin_vec::ThinVec<::nsstring::nsCString>, _retval: *mut thin_vec::ThinVec<::nsstring::nsCString>) -> ::nserror::nsresult,

    /* Array<jsval> testJsvalSequence (in Array<jsval> a, inout Array<jsval> b); */
    /// Unable to generate binding because `specialtype jsval unsupported`
    pub TestJsvalSequence: *const ::libc::c_void,

    /* Array<Array<short>> testSequenceSequence (in Array<Array<short>> a, inout Array<Array<short>> b); */
    pub TestSequenceSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *const thin_vec::ThinVec<thin_vec::ThinVec<i16>>, b: *mut thin_vec::ThinVec<thin_vec::ThinVec<i16>>, _retval: *mut thin_vec::ThinVec<thin_vec::ThinVec<i16>>) -> ::nserror::nsresult,

    /* void testInterfaceIsSequence (in nsIIDPtr aIID, [iid_is (aIID)] in Array<nsQIResult> a, inout nsIIDPtr bIID, [iid_is (bIID)] inout Array<nsQIResult> b, out nsIIDPtr rvIID, [iid_is (rvIID), retval] out Array<nsQIResult> rv); */
    pub TestInterfaceIsSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, aIID: *const nsIID, a: *const thin_vec::ThinVec<*const libc::c_void>, bIID: *mut *mut nsIID, b: *mut thin_vec::ThinVec<*const libc::c_void>, rvIID: *mut *mut nsIID, rv: *mut thin_vec::ThinVec<*const libc::c_void>) -> ::nserror::nsresult,

    /* Array<uint8_t> testOptionalSequence ([optional] in Array<uint8_t> arr); */
    pub TestOptionalSequence: unsafe extern "system" fn (this: *const nsIXPCTestParams, arr: *const thin_vec::ThinVec<uint8_t>, _retval: *mut thin_vec::ThinVec<uint8_t>) -> ::nserror::nsresult,

    /* void testShortArray (in unsigned long aLength, [array, size_is (aLength)] in short a, inout unsigned long bLength, [array, size_is (bLength)] inout short b, out unsigned long rvLength, [array, size_is (rvLength), retval] out short rv); */
    pub TestShortArray: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *mut i16, bLength: *mut u32, b: *mut *mut i16, rvLength: *mut u32, rv: *mut *mut i16) -> ::nserror::nsresult,

    /* void testDoubleArray (in unsigned long aLength, [array, size_is (aLength)] in double a, inout unsigned long bLength, [array, size_is (bLength)] inout double b, out unsigned long rvLength, [array, size_is (rvLength), retval] out double rv); */
    pub TestDoubleArray: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *mut libc::c_double, bLength: *mut u32, b: *mut *mut libc::c_double, rvLength: *mut u32, rv: *mut *mut libc::c_double) -> ::nserror::nsresult,

    /* void testStringArray (in unsigned long aLength, [array, size_is (aLength)] in string a, inout unsigned long bLength, [array, size_is (bLength)] inout string b, out unsigned long rvLength, [array, size_is (rvLength), retval] out string rv); */
    pub TestStringArray: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *mut *const libc::c_char, bLength: *mut u32, b: *mut *mut *const libc::c_char, rvLength: *mut u32, rv: *mut *mut *const libc::c_char) -> ::nserror::nsresult,

    /* void testWstringArray (in unsigned long aLength, [array, size_is (aLength)] in wstring a, inout unsigned long bLength, [array, size_is (bLength)] inout wstring b, out unsigned long rvLength, [array, size_is (rvLength), retval] out wstring rv); */
    pub TestWstringArray: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *mut *const i16, bLength: *mut u32, b: *mut *mut *const i16, rvLength: *mut u32, rv: *mut *mut *const i16) -> ::nserror::nsresult,

    /* void testInterfaceArray (in unsigned long aLength, [array, size_is (aLength)] in nsIXPCTestInterfaceA a, inout unsigned long bLength, [array, size_is (bLength)] inout nsIXPCTestInterfaceA b, out unsigned long rvLength, [array, size_is (rvLength), retval] out nsIXPCTestInterfaceA rv); */
    pub TestInterfaceArray: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *mut *const nsIXPCTestInterfaceA, bLength: *mut u32, b: *mut *mut *const nsIXPCTestInterfaceA, rvLength: *mut u32, rv: *mut *mut *const nsIXPCTestInterfaceA) -> ::nserror::nsresult,

    /* unsigned long testByteArrayOptionalLength ([array, size_is (aLength)] in uint8_t a, [optional] in unsigned long aLength); */
    pub TestByteArrayOptionalLength: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *mut uint8_t, aLength: u32, _retval: *mut u32) -> ::nserror::nsresult,

    /* void testSizedString (in unsigned long aLength, [size_is (aLength)] in string a, inout unsigned long bLength, [size_is (bLength)] inout string b, out unsigned long rvLength, [size_is (rvLength), retval] out string rv); */
    pub TestSizedString: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *const libc::c_char, bLength: *mut u32, b: *mut *const libc::c_char, rvLength: *mut u32, rv: *mut *const libc::c_char) -> ::nserror::nsresult,

    /* void testSizedWstring (in unsigned long aLength, [size_is (aLength)] in wstring a, inout unsigned long bLength, [size_is (bLength)] inout wstring b, out unsigned long rvLength, [size_is (rvLength), retval] out wstring rv); */
    pub TestSizedWstring: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, a: *const i16, bLength: *mut u32, b: *mut *const i16, rvLength: *mut u32, rv: *mut *const i16) -> ::nserror::nsresult,

    /* void testInterfaceIs (in nsIIDPtr aIID, [iid_is (aIID)] in nsQIResult a, inout nsIIDPtr bIID, [iid_is (bIID)] inout nsQIResult b, out nsIIDPtr rvIID, [iid_is (rvIID), retval] out nsQIResult rv); */
    pub TestInterfaceIs: unsafe extern "system" fn (this: *const nsIXPCTestParams, aIID: *const nsIID, a: *const libc::c_void, bIID: *mut *mut nsIID, b: *mut *mut libc::c_void, rvIID: *mut *mut nsIID, rv: *mut *mut libc::c_void) -> ::nserror::nsresult,

    /* void testInterfaceIsArray (in unsigned long aLength, in nsIIDPtr aIID, [array, size_is (aLength), iid_is (aIID)] in nsQIResult a, inout unsigned long bLength, inout nsIIDPtr bIID, [array, size_is (bLength), iid_is (bIID)] inout nsQIResult b, out unsigned long rvLength, out nsIIDPtr rvIID, [retval, array, size_is (rvLength), iid_is (rvIID)] out nsQIResult rv); */
    pub TestInterfaceIsArray: unsafe extern "system" fn (this: *const nsIXPCTestParams, aLength: u32, aIID: *const nsIID, a: *mut *const libc::c_void, bLength: *mut u32, bIID: *mut *mut nsIID, b: *mut *mut *const libc::c_void, rvLength: *mut u32, rvIID: *mut *mut nsIID, rv: *mut *mut *const libc::c_void) -> ::nserror::nsresult,

    /* void testJsvalArray (in unsigned long aLength, [array, size_is (aLength)] in jsval a, inout unsigned long bLength, [array, size_is (bLength)] inout jsval b, out unsigned long rvLength, [array, size_is (rvLength), retval] out jsval rv); */
    /// Unable to generate binding because `specialtype jsval unsupported`
    pub TestJsvalArray: *const ::libc::c_void,

    /* void testOutAString (out AString o); */
    pub TestOutAString: unsafe extern "system" fn (this: *const nsIXPCTestParams, o: *mut ::nsstring::nsAString) -> ::nserror::nsresult,

    /* ACString testStringArrayOptionalSize ([array, size_is (aLength)] in string a, [optional] in unsigned long aLength); */
    pub TestStringArrayOptionalSize: unsafe extern "system" fn (this: *const nsIXPCTestParams, a: *mut *const libc::c_char, aLength: u32, _retval: *mut ::nsstring::nsACString) -> ::nserror::nsresult,

    /* void testOmittedOptionalOut ([optional] out nsIURI aOut); */
    pub TestOmittedOptionalOut: unsafe extern "system" fn (this: *const nsIXPCTestParams, aOut: *mut*const nsIURI) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestParams {


    /// `boolean testBoolean (in boolean a, inout boolean b);`
    #[inline]
    pub unsafe fn TestBoolean(&self, a: bool, b: *mut bool, _retval: *mut bool) -> ::nserror::nsresult {
        ((*self.vtable).TestBoolean)(self, a, b, _retval)
    }



    /// `octet testOctet (in octet a, inout octet b);`
    #[inline]
    pub unsafe fn TestOctet(&self, a: u8, b: *mut u8, _retval: *mut u8) -> ::nserror::nsresult {
        ((*self.vtable).TestOctet)(self, a, b, _retval)
    }



    /// `short testShort (in short a, inout short b);`
    #[inline]
    pub unsafe fn TestShort(&self, a: i16, b: *mut i16, _retval: *mut i16) -> ::nserror::nsresult {
        ((*self.vtable).TestShort)(self, a, b, _retval)
    }



    /// `long testLong (in long a, inout long b);`
    #[inline]
    pub unsafe fn TestLong(&self, a: i32, b: *mut i32, _retval: *mut i32) -> ::nserror::nsresult {
        ((*self.vtable).TestLong)(self, a, b, _retval)
    }



    /// `long long testLongLong (in long long a, inout long long b);`
    #[inline]
    pub unsafe fn TestLongLong(&self, a: i64, b: *mut i64, _retval: *mut i64) -> ::nserror::nsresult {
        ((*self.vtable).TestLongLong)(self, a, b, _retval)
    }



    /// `unsigned short testUnsignedShort (in unsigned short a, inout unsigned short b);`
    #[inline]
    pub unsafe fn TestUnsignedShort(&self, a: u16, b: *mut u16, _retval: *mut u16) -> ::nserror::nsresult {
        ((*self.vtable).TestUnsignedShort)(self, a, b, _retval)
    }



    /// `unsigned long testUnsignedLong (in unsigned long a, inout unsigned long b);`
    #[inline]
    pub unsafe fn TestUnsignedLong(&self, a: u32, b: *mut u32, _retval: *mut u32) -> ::nserror::nsresult {
        ((*self.vtable).TestUnsignedLong)(self, a, b, _retval)
    }



    /// `unsigned long long testUnsignedLongLong (in unsigned long long a, inout unsigned long long b);`
    #[inline]
    pub unsafe fn TestUnsignedLongLong(&self, a: u64, b: *mut u64, _retval: *mut u64) -> ::nserror::nsresult {
        ((*self.vtable).TestUnsignedLongLong)(self, a, b, _retval)
    }



    /// `float testFloat (in float a, inout float b);`
    #[inline]
    pub unsafe fn TestFloat(&self, a: libc::c_float, b: *mut libc::c_float, _retval: *mut libc::c_float) -> ::nserror::nsresult {
        ((*self.vtable).TestFloat)(self, a, b, _retval)
    }



    /// `double testDouble (in double a, inout float b);`
    #[inline]
    pub unsafe fn TestDouble(&self, a: libc::c_double, b: *mut libc::c_float, _retval: *mut libc::c_double) -> ::nserror::nsresult {
        ((*self.vtable).TestDouble)(self, a, b, _retval)
    }



    /// `char testChar (in char a, inout char b);`
    #[inline]
    pub unsafe fn TestChar(&self, a: libc::c_char, b: *mut libc::c_char, _retval: *mut libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).TestChar)(self, a, b, _retval)
    }



    /// `string testString (in string a, inout string b);`
    #[inline]
    pub unsafe fn TestString(&self, a: *const libc::c_char, b: *mut *const libc::c_char, _retval: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).TestString)(self, a, b, _retval)
    }



    /// `wchar testWchar (in wchar a, inout wchar b);`
    #[inline]
    pub unsafe fn TestWchar(&self, a: i16, b: *mut i16, _retval: *mut i16) -> ::nserror::nsresult {
        ((*self.vtable).TestWchar)(self, a, b, _retval)
    }



    /// `wstring testWstring (in wstring a, inout wstring b);`
    #[inline]
    pub unsafe fn TestWstring(&self, a: *const i16, b: *mut *const i16, _retval: *mut *const i16) -> ::nserror::nsresult {
        ((*self.vtable).TestWstring)(self, a, b, _retval)
    }



    /// `AString testAString (in AString a, inout AString b);`
    #[inline]
    pub unsafe fn TestAString(&self, a: *const ::nsstring::nsAString, b: *mut ::nsstring::nsAString, _retval: *mut ::nsstring::nsAString) -> ::nserror::nsresult {
        ((*self.vtable).TestAString)(self, a, b, _retval)
    }



    /// `AUTF8String testAUTF8String (in AUTF8String a, inout AUTF8String b);`
    #[inline]
    pub unsafe fn TestAUTF8String(&self, a: *const ::nsstring::nsACString, b: *mut ::nsstring::nsACString, _retval: *mut ::nsstring::nsACString) -> ::nserror::nsresult {
        ((*self.vtable).TestAUTF8String)(self, a, b, _retval)
    }



    /// `ACString testACString (in ACString a, inout ACString b);`
    #[inline]
    pub unsafe fn TestACString(&self, a: *const ::nsstring::nsACString, b: *mut ::nsstring::nsACString, _retval: *mut ::nsstring::nsACString) -> ::nserror::nsresult {
        ((*self.vtable).TestACString)(self, a, b, _retval)
    }



    /// `jsval testJsval (in jsval a, inout jsval b);`
    const _TestJsval: () = ();


    /// `Array<short> testShortSequence (in Array<short> a, inout Array<short> b);`
    #[inline]
    pub unsafe fn TestShortSequence(&self, a: *const thin_vec::ThinVec<i16>, b: *mut thin_vec::ThinVec<i16>, _retval: *mut thin_vec::ThinVec<i16>) -> ::nserror::nsresult {
        ((*self.vtable).TestShortSequence)(self, a, b, _retval)
    }



    /// `Array<double> testDoubleSequence (in Array<double> a, inout Array<double> b);`
    #[inline]
    pub unsafe fn TestDoubleSequence(&self, a: *const thin_vec::ThinVec<libc::c_double>, b: *mut thin_vec::ThinVec<libc::c_double>, _retval: *mut thin_vec::ThinVec<libc::c_double>) -> ::nserror::nsresult {
        ((*self.vtable).TestDoubleSequence)(self, a, b, _retval)
    }



    /// `Array<nsIXPCTestInterfaceA> testInterfaceSequence (in Array<nsIXPCTestInterfaceA> a, inout Array<nsIXPCTestInterfaceA> b);`
    #[inline]
    pub unsafe fn TestInterfaceSequence(&self, a: *const thin_vec::ThinVec<RefPtr<nsIXPCTestInterfaceA>>, b: *mut thin_vec::ThinVec<RefPtr<nsIXPCTestInterfaceA>>, _retval: *mut thin_vec::ThinVec<RefPtr<nsIXPCTestInterfaceA>>) -> ::nserror::nsresult {
        ((*self.vtable).TestInterfaceSequence)(self, a, b, _retval)
    }



    /// `Array<AString> testAStringSequence (in Array<AString> a, inout Array<AString> b);`
    #[inline]
    pub unsafe fn TestAStringSequence(&self, a: *const thin_vec::ThinVec<::nsstring::nsString>, b: *mut thin_vec::ThinVec<::nsstring::nsString>, _retval: *mut thin_vec::ThinVec<::nsstring::nsString>) -> ::nserror::nsresult {
        ((*self.vtable).TestAStringSequence)(self, a, b, _retval)
    }



    /// `Array<ACString> testACStringSequence (in Array<ACString> a, inout Array<ACString> b);`
    #[inline]
    pub unsafe fn TestACStringSequence(&self, a: *const thin_vec::ThinVec<::nsstring::nsCString>, b: *mut thin_vec::ThinVec<::nsstring::nsCString>, _retval: *mut thin_vec::ThinVec<::nsstring::nsCString>) -> ::nserror::nsresult {
        ((*self.vtable).TestACStringSequence)(self, a, b, _retval)
    }



    /// `Array<jsval> testJsvalSequence (in Array<jsval> a, inout Array<jsval> b);`
    const _TestJsvalSequence: () = ();


    /// `Array<Array<short>> testSequenceSequence (in Array<Array<short>> a, inout Array<Array<short>> b);`
    #[inline]
    pub unsafe fn TestSequenceSequence(&self, a: *const thin_vec::ThinVec<thin_vec::ThinVec<i16>>, b: *mut thin_vec::ThinVec<thin_vec::ThinVec<i16>>, _retval: *mut thin_vec::ThinVec<thin_vec::ThinVec<i16>>) -> ::nserror::nsresult {
        ((*self.vtable).TestSequenceSequence)(self, a, b, _retval)
    }



    /// `void testInterfaceIsSequence (in nsIIDPtr aIID, [iid_is (aIID)] in Array<nsQIResult> a, inout nsIIDPtr bIID, [iid_is (bIID)] inout Array<nsQIResult> b, out nsIIDPtr rvIID, [iid_is (rvIID), retval] out Array<nsQIResult> rv);`
    #[inline]
    pub unsafe fn TestInterfaceIsSequence(&self, aIID: *const nsIID, a: *const thin_vec::ThinVec<*const libc::c_void>, bIID: *mut *mut nsIID, b: *mut thin_vec::ThinVec<*const libc::c_void>, rvIID: *mut *mut nsIID, rv: *mut thin_vec::ThinVec<*const libc::c_void>) -> ::nserror::nsresult {
        ((*self.vtable).TestInterfaceIsSequence)(self, aIID, a, bIID, b, rvIID, rv)
    }



    /// `Array<uint8_t> testOptionalSequence ([optional] in Array<uint8_t> arr);`
    #[inline]
    pub unsafe fn TestOptionalSequence(&self, arr: *const thin_vec::ThinVec<uint8_t>, _retval: *mut thin_vec::ThinVec<uint8_t>) -> ::nserror::nsresult {
        ((*self.vtable).TestOptionalSequence)(self, arr, _retval)
    }



    /// `void testShortArray (in unsigned long aLength, [array, size_is (aLength)] in short a, inout unsigned long bLength, [array, size_is (bLength)] inout short b, out unsigned long rvLength, [array, size_is (rvLength), retval] out short rv);`
    #[inline]
    pub unsafe fn TestShortArray(&self, aLength: u32, a: *mut i16, bLength: *mut u32, b: *mut *mut i16, rvLength: *mut u32, rv: *mut *mut i16) -> ::nserror::nsresult {
        ((*self.vtable).TestShortArray)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `void testDoubleArray (in unsigned long aLength, [array, size_is (aLength)] in double a, inout unsigned long bLength, [array, size_is (bLength)] inout double b, out unsigned long rvLength, [array, size_is (rvLength), retval] out double rv);`
    #[inline]
    pub unsafe fn TestDoubleArray(&self, aLength: u32, a: *mut libc::c_double, bLength: *mut u32, b: *mut *mut libc::c_double, rvLength: *mut u32, rv: *mut *mut libc::c_double) -> ::nserror::nsresult {
        ((*self.vtable).TestDoubleArray)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `void testStringArray (in unsigned long aLength, [array, size_is (aLength)] in string a, inout unsigned long bLength, [array, size_is (bLength)] inout string b, out unsigned long rvLength, [array, size_is (rvLength), retval] out string rv);`
    #[inline]
    pub unsafe fn TestStringArray(&self, aLength: u32, a: *mut *const libc::c_char, bLength: *mut u32, b: *mut *mut *const libc::c_char, rvLength: *mut u32, rv: *mut *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).TestStringArray)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `void testWstringArray (in unsigned long aLength, [array, size_is (aLength)] in wstring a, inout unsigned long bLength, [array, size_is (bLength)] inout wstring b, out unsigned long rvLength, [array, size_is (rvLength), retval] out wstring rv);`
    #[inline]
    pub unsafe fn TestWstringArray(&self, aLength: u32, a: *mut *const i16, bLength: *mut u32, b: *mut *mut *const i16, rvLength: *mut u32, rv: *mut *mut *const i16) -> ::nserror::nsresult {
        ((*self.vtable).TestWstringArray)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `void testInterfaceArray (in unsigned long aLength, [array, size_is (aLength)] in nsIXPCTestInterfaceA a, inout unsigned long bLength, [array, size_is (bLength)] inout nsIXPCTestInterfaceA b, out unsigned long rvLength, [array, size_is (rvLength), retval] out nsIXPCTestInterfaceA rv);`
    #[inline]
    pub unsafe fn TestInterfaceArray(&self, aLength: u32, a: *mut *const nsIXPCTestInterfaceA, bLength: *mut u32, b: *mut *mut *const nsIXPCTestInterfaceA, rvLength: *mut u32, rv: *mut *mut *const nsIXPCTestInterfaceA) -> ::nserror::nsresult {
        ((*self.vtable).TestInterfaceArray)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `unsigned long testByteArrayOptionalLength ([array, size_is (aLength)] in uint8_t a, [optional] in unsigned long aLength);`
    #[inline]
    pub unsafe fn TestByteArrayOptionalLength(&self, a: *mut uint8_t, aLength: u32, _retval: *mut u32) -> ::nserror::nsresult {
        ((*self.vtable).TestByteArrayOptionalLength)(self, a, aLength, _retval)
    }



    /// `void testSizedString (in unsigned long aLength, [size_is (aLength)] in string a, inout unsigned long bLength, [size_is (bLength)] inout string b, out unsigned long rvLength, [size_is (rvLength), retval] out string rv);`
    #[inline]
    pub unsafe fn TestSizedString(&self, aLength: u32, a: *const libc::c_char, bLength: *mut u32, b: *mut *const libc::c_char, rvLength: *mut u32, rv: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).TestSizedString)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `void testSizedWstring (in unsigned long aLength, [size_is (aLength)] in wstring a, inout unsigned long bLength, [size_is (bLength)] inout wstring b, out unsigned long rvLength, [size_is (rvLength), retval] out wstring rv);`
    #[inline]
    pub unsafe fn TestSizedWstring(&self, aLength: u32, a: *const i16, bLength: *mut u32, b: *mut *const i16, rvLength: *mut u32, rv: *mut *const i16) -> ::nserror::nsresult {
        ((*self.vtable).TestSizedWstring)(self, aLength, a, bLength, b, rvLength, rv)
    }



    /// `void testInterfaceIs (in nsIIDPtr aIID, [iid_is (aIID)] in nsQIResult a, inout nsIIDPtr bIID, [iid_is (bIID)] inout nsQIResult b, out nsIIDPtr rvIID, [iid_is (rvIID), retval] out nsQIResult rv);`
    #[inline]
    pub unsafe fn TestInterfaceIs(&self, aIID: *const nsIID, a: *const libc::c_void, bIID: *mut *mut nsIID, b: *mut *mut libc::c_void, rvIID: *mut *mut nsIID, rv: *mut *mut libc::c_void) -> ::nserror::nsresult {
        ((*self.vtable).TestInterfaceIs)(self, aIID, a, bIID, b, rvIID, rv)
    }



    /// `void testInterfaceIsArray (in unsigned long aLength, in nsIIDPtr aIID, [array, size_is (aLength), iid_is (aIID)] in nsQIResult a, inout unsigned long bLength, inout nsIIDPtr bIID, [array, size_is (bLength), iid_is (bIID)] inout nsQIResult b, out unsigned long rvLength, out nsIIDPtr rvIID, [retval, array, size_is (rvLength), iid_is (rvIID)] out nsQIResult rv);`
    #[inline]
    pub unsafe fn TestInterfaceIsArray(&self, aLength: u32, aIID: *const nsIID, a: *mut *const libc::c_void, bLength: *mut u32, bIID: *mut *mut nsIID, b: *mut *mut *const libc::c_void, rvLength: *mut u32, rvIID: *mut *mut nsIID, rv: *mut *mut *const libc::c_void) -> ::nserror::nsresult {
        ((*self.vtable).TestInterfaceIsArray)(self, aLength, aIID, a, bLength, bIID, b, rvLength, rvIID, rv)
    }



    /// `void testJsvalArray (in unsigned long aLength, [array, size_is (aLength)] in jsval a, inout unsigned long bLength, [array, size_is (bLength)] inout jsval b, out unsigned long rvLength, [array, size_is (rvLength), retval] out jsval rv);`
    const _TestJsvalArray: () = ();


    /// `void testOutAString (out AString o);`
    #[inline]
    pub unsafe fn TestOutAString(&self, o: *mut ::nsstring::nsAString) -> ::nserror::nsresult {
        ((*self.vtable).TestOutAString)(self, o)
    }



    /// `ACString testStringArrayOptionalSize ([array, size_is (aLength)] in string a, [optional] in unsigned long aLength);`
    #[inline]
    pub unsafe fn TestStringArrayOptionalSize(&self, a: *mut *const libc::c_char, aLength: u32, _retval: *mut ::nsstring::nsACString) -> ::nserror::nsresult {
        ((*self.vtable).TestStringArrayOptionalSize)(self, a, aLength, _retval)
    }



    /// `void testOmittedOptionalOut ([optional] out nsIURI aOut);`
    #[inline]
    pub unsafe fn TestOmittedOptionalOut(&self, aOut: *mut*const nsIURI) -> ::nserror::nsresult {
        ((*self.vtable).TestOmittedOptionalOut)(self, aOut)
    }


}


