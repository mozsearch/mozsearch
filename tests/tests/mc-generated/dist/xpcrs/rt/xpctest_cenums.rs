//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_cenums.idl
//


/// `interface nsIXPCTestCEnums : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestCEnums {
    vtable: *const nsIXPCTestCEnumsVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestCEnums.
unsafe impl XpCom for nsIXPCTestCEnums {
    const IID: nsIID = nsID(0x6a2f918e, 0xcda2, 0x11e8,
        [0xbc, 0x9a, 0xa3, 0x4c, 0x71, 0x6d, 0x1f, 0x2a]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestCEnums {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestCEnums.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestCEnumsCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestCEnums`.
    fn coerce_from(v: &nsIXPCTestCEnums) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestCEnumsCoerce for nsIXPCTestCEnums {
    #[inline]
    fn coerce_from(v: &nsIXPCTestCEnums) -> &Self {
        v
    }
}

impl nsIXPCTestCEnums {
    /// Cast this `nsIXPCTestCEnums` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestCEnumsCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestCEnums {
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
impl<T: nsISupportsCoerce> nsIXPCTestCEnumsCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestCEnums) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestCEnums
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestCEnumsVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* void testCEnumInput (in nsIXPCTestCEnums_testFlagsExplicit abc); */
    pub TestCEnumInput: unsafe extern "system" fn (this: *const nsIXPCTestCEnums, abc:  u8) -> ::nserror::nsresult,

    /* nsIXPCTestCEnums_testFlagsExplicit testCEnumOutput (); */
    pub TestCEnumOutput: unsafe extern "system" fn (this: *const nsIXPCTestCEnums, _retval: *mut u8) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestCEnums {

    pub const testConst: i32 = 1;


    /// `void testCEnumInput (in nsIXPCTestCEnums_testFlagsExplicit abc);`
    #[inline]
    pub unsafe fn TestCEnumInput(&self, abc:  u8) -> ::nserror::nsresult {
        ((*self.vtable).TestCEnumInput)(self, abc)
    }



    /// `nsIXPCTestCEnums_testFlagsExplicit testCEnumOutput ();`
    #[inline]
    pub unsafe fn TestCEnumOutput(&self, _retval: *mut u8) -> ::nserror::nsresult {
        ((*self.vtable).TestCEnumOutput)(self, _retval)
    }


}


