//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_returncode.idl
//


/// `interface nsIXPCTestReturnCodeParent : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestReturnCodeParent {
    vtable: *const nsIXPCTestReturnCodeParentVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestReturnCodeParent.
unsafe impl XpCom for nsIXPCTestReturnCodeParent {
    const IID: nsIID = nsID(0x479e4532, 0x95cf, 0x48b8,
        [0xa9, 0x9b, 0x8a, 0x58, 0x81, 0xe4, 0x71, 0x38]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestReturnCodeParent {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestReturnCodeParent.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestReturnCodeParentCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestReturnCodeParent`.
    fn coerce_from(v: &nsIXPCTestReturnCodeParent) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestReturnCodeParentCoerce for nsIXPCTestReturnCodeParent {
    #[inline]
    fn coerce_from(v: &nsIXPCTestReturnCodeParent) -> &Self {
        v
    }
}

impl nsIXPCTestReturnCodeParent {
    /// Cast this `nsIXPCTestReturnCodeParent` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestReturnCodeParentCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestReturnCodeParent {
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
impl<T: nsISupportsCoerce> nsIXPCTestReturnCodeParentCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestReturnCodeParent) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestReturnCodeParent
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestReturnCodeParentVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* nsresult callChild (in long childBehavior); */
    pub CallChild: unsafe extern "system" fn (this: *const nsIXPCTestReturnCodeParent, childBehavior: i32, _retval: *mut ::nserror::nsresult) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestReturnCodeParent {


    /// `nsresult callChild (in long childBehavior);`
    #[inline]
    pub unsafe fn CallChild(&self, childBehavior: i32, _retval: *mut ::nserror::nsresult) -> ::nserror::nsresult {
        ((*self.vtable).CallChild)(self, childBehavior, _retval)
    }


}


/// `interface nsIXPCTestReturnCodeChild : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestReturnCodeChild {
    vtable: *const nsIXPCTestReturnCodeChildVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestReturnCodeChild.
unsafe impl XpCom for nsIXPCTestReturnCodeChild {
    const IID: nsIID = nsID(0x672cfd34, 0x1fd1, 0x455d,
        [0x99, 0x01, 0xd8, 0x79, 0xfa, 0x6f, 0xdb, 0x95]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestReturnCodeChild {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestReturnCodeChild.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestReturnCodeChildCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestReturnCodeChild`.
    fn coerce_from(v: &nsIXPCTestReturnCodeChild) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestReturnCodeChildCoerce for nsIXPCTestReturnCodeChild {
    #[inline]
    fn coerce_from(v: &nsIXPCTestReturnCodeChild) -> &Self {
        v
    }
}

impl nsIXPCTestReturnCodeChild {
    /// Cast this `nsIXPCTestReturnCodeChild` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestReturnCodeChildCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestReturnCodeChild {
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
impl<T: nsISupportsCoerce> nsIXPCTestReturnCodeChildCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestReturnCodeChild) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestReturnCodeChild
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestReturnCodeChildVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* void doIt (in long behavior); */
    pub DoIt: unsafe extern "system" fn (this: *const nsIXPCTestReturnCodeChild, behavior: i32) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestReturnCodeChild {

    pub const CHILD_SHOULD_THROW: i32 = 0;


    pub const CHILD_SHOULD_RETURN_SUCCESS: i32 = 1;


    pub const CHILD_SHOULD_RETURN_RESULTCODE: i32 = 2;


    pub const CHILD_SHOULD_NEST_RESULTCODES: i32 = 3;


    /// `void doIt (in long behavior);`
    #[inline]
    pub unsafe fn DoIt(&self, behavior: i32) -> ::nserror::nsresult {
        ((*self.vtable).DoIt)(self, behavior)
    }


}


