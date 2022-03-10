//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_utils.idl
//


/// `interface nsIXPCTestFunctionInterface : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestFunctionInterface {
    vtable: *const nsIXPCTestFunctionInterfaceVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestFunctionInterface.
unsafe impl XpCom for nsIXPCTestFunctionInterface {
    const IID: nsIID = nsID(0xd58a82ab, 0xd8f7, 0x4ca9,
        [0x92, 0x73, 0xb3, 0x29, 0x0d, 0x42, 0xa0, 0xcf]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestFunctionInterface {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestFunctionInterface.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestFunctionInterfaceCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestFunctionInterface`.
    fn coerce_from(v: &nsIXPCTestFunctionInterface) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestFunctionInterfaceCoerce for nsIXPCTestFunctionInterface {
    #[inline]
    fn coerce_from(v: &nsIXPCTestFunctionInterface) -> &Self {
        v
    }
}

impl nsIXPCTestFunctionInterface {
    /// Cast this `nsIXPCTestFunctionInterface` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestFunctionInterfaceCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestFunctionInterface {
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
impl<T: nsISupportsCoerce> nsIXPCTestFunctionInterfaceCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestFunctionInterface) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestFunctionInterface
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestFunctionInterfaceVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* string echo (in string arg); */
    pub Echo: unsafe extern "system" fn (this: *const nsIXPCTestFunctionInterface, arg: *const libc::c_char, _retval: *mut *const libc::c_char) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestFunctionInterface {


    /// `string echo (in string arg);`
    #[inline]
    pub unsafe fn Echo(&self, arg: *const libc::c_char, _retval: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).Echo)(self, arg, _retval)
    }


}


/// `interface nsIXPCTestUtils : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestUtils {
    vtable: *const nsIXPCTestUtilsVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestUtils.
unsafe impl XpCom for nsIXPCTestUtils {
    const IID: nsIID = nsID(0x1e9cddeb, 0x510d, 0x449a,
        [0xb1, 0x52, 0x3c, 0x1b, 0x5b, 0x31, 0xd4, 0x1d]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestUtils {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestUtils.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestUtilsCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestUtils`.
    fn coerce_from(v: &nsIXPCTestUtils) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestUtilsCoerce for nsIXPCTestUtils {
    #[inline]
    fn coerce_from(v: &nsIXPCTestUtils) -> &Self {
        v
    }
}

impl nsIXPCTestUtils {
    /// Cast this `nsIXPCTestUtils` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestUtilsCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestUtils {
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
impl<T: nsISupportsCoerce> nsIXPCTestUtilsCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestUtils) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestUtils
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestUtilsVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f); */
    pub DoubleWrapFunction: unsafe extern "system" fn (this: *const nsIXPCTestUtils, f: *const nsIXPCTestFunctionInterface, _retval: *mut *const nsIXPCTestFunctionInterface) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestUtils {


    /// `nsIXPCTestFunctionInterface doubleWrapFunction (in nsIXPCTestFunctionInterface f);`
    #[inline]
    pub unsafe fn DoubleWrapFunction(&self, f: *const nsIXPCTestFunctionInterface, _retval: *mut *const nsIXPCTestFunctionInterface) -> ::nserror::nsresult {
        ((*self.vtable).DoubleWrapFunction)(self, f, _retval)
    }


}


