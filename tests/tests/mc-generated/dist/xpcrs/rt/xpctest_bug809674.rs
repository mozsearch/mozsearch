//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_bug809674.idl
//


/// `interface nsIXPCTestBug809674 : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestBug809674 {
    vtable: *const nsIXPCTestBug809674VTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestBug809674.
unsafe impl XpCom for nsIXPCTestBug809674 {
    const IID: nsIID = nsID(0x2df46559, 0xda21, 0x49bf,
        [0xb8, 0x63, 0x0d, 0x7b, 0x7b, 0xbc, 0xbc, 0x73]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestBug809674 {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestBug809674.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestBug809674Coerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestBug809674`.
    fn coerce_from(v: &nsIXPCTestBug809674) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestBug809674Coerce for nsIXPCTestBug809674 {
    #[inline]
    fn coerce_from(v: &nsIXPCTestBug809674) -> &Self {
        v
    }
}

impl nsIXPCTestBug809674 {
    /// Cast this `nsIXPCTestBug809674` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestBug809674Coerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestBug809674 {
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
impl<T: nsISupportsCoerce> nsIXPCTestBug809674Coerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestBug809674) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestBug809674
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestBug809674VTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* [implicit_jscontext] unsigned long addArgs (in unsigned long x, in unsigned long y); */
    /// Unable to generate binding because `jscontext is unsupported`
    pub AddArgs: *const ::libc::c_void,

    /* [implicit_jscontext] unsigned long addSubMulArgs (in unsigned long x, in unsigned long y, out unsigned long subOut, out unsigned long mulOut); */
    /// Unable to generate binding because `jscontext is unsupported`
    pub AddSubMulArgs: *const ::libc::c_void,

    /* [implicit_jscontext] jsval addVals (in jsval x, in jsval y); */
    /// Unable to generate binding because `specialtype jsval unsupported`
    pub AddVals: *const ::libc::c_void,

    /* [implicit_jscontext] unsigned long methodNoArgs (); */
    /// Unable to generate binding because `jscontext is unsupported`
    pub MethodNoArgs: *const ::libc::c_void,

    /* [implicit_jscontext] void methodNoArgsNoRetVal (); */
    /// Unable to generate binding because `jscontext is unsupported`
    pub MethodNoArgsNoRetVal: *const ::libc::c_void,

    /* [implicit_jscontext] unsigned long addMany (in unsigned long x1, in unsigned long x2, in unsigned long x3, in unsigned long x4, in unsigned long x5, in unsigned long x6, in unsigned long x7, in unsigned long x8); */
    /// Unable to generate binding because `jscontext is unsupported`
    pub AddMany: *const ::libc::c_void,

    /* [implicit_jscontext] attribute jsval valProperty; */
    /// Unable to generate binding because `specialtype jsval unsupported`
    pub GetValProperty: *const ::libc::c_void,

    /* [implicit_jscontext] attribute jsval valProperty; */
    /// Unable to generate binding because `specialtype jsval unsupported`
    pub SetValProperty: *const ::libc::c_void,

    /* [implicit_jscontext] attribute unsigned long uintProperty; */
    /// Unable to generate binding because `jscontext is unsupported`
    pub GetUintProperty: *const ::libc::c_void,

    /* [implicit_jscontext] attribute unsigned long uintProperty; */
    /// Unable to generate binding because `jscontext is unsupported`
    pub SetUintProperty: *const ::libc::c_void,

    /* [optional_argc] void methodWithOptionalArgc (); */
    /// Unable to generate binding because `optional_argc is unsupported`
    pub MethodWithOptionalArgc: *const ::libc::c_void,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestBug809674 {


    /// `[implicit_jscontext] unsigned long addArgs (in unsigned long x, in unsigned long y);`
    const _AddArgs: () = ();


    /// `[implicit_jscontext] unsigned long addSubMulArgs (in unsigned long x, in unsigned long y, out unsigned long subOut, out unsigned long mulOut);`
    const _AddSubMulArgs: () = ();


    /// `[implicit_jscontext] jsval addVals (in jsval x, in jsval y);`
    const _AddVals: () = ();


    /// `[implicit_jscontext] unsigned long methodNoArgs ();`
    const _MethodNoArgs: () = ();


    /// `[implicit_jscontext] void methodNoArgsNoRetVal ();`
    const _MethodNoArgsNoRetVal: () = ();


    /// `[implicit_jscontext] unsigned long addMany (in unsigned long x1, in unsigned long x2, in unsigned long x3, in unsigned long x4, in unsigned long x5, in unsigned long x6, in unsigned long x7, in unsigned long x8);`
    const _AddMany: () = ();


    /// `[implicit_jscontext] attribute jsval valProperty;`
    const _GetValProperty: () = ();


    /// `[implicit_jscontext] attribute jsval valProperty;`
    const _SetValProperty: () = ();


    /// `[implicit_jscontext] attribute unsigned long uintProperty;`
    const _GetUintProperty: () = ();


    /// `[implicit_jscontext] attribute unsigned long uintProperty;`
    const _SetUintProperty: () = ();


    /// `[optional_argc] void methodWithOptionalArgc ();`
    const _MethodWithOptionalArgc: () = ();

}


