//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_interfaces.idl
//


/// `interface nsIXPCTestInterfaceA : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestInterfaceA {
    vtable: *const nsIXPCTestInterfaceAVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestInterfaceA.
unsafe impl XpCom for nsIXPCTestInterfaceA {
    const IID: nsIID = nsID(0x3c8fd2f5, 0x970c, 0x42c6,
        [0xb5, 0xdd, 0xcd, 0xa1, 0xc1, 0x6d, 0xcf, 0xd8]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestInterfaceA {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestInterfaceA.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestInterfaceACoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestInterfaceA`.
    fn coerce_from(v: &nsIXPCTestInterfaceA) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestInterfaceACoerce for nsIXPCTestInterfaceA {
    #[inline]
    fn coerce_from(v: &nsIXPCTestInterfaceA) -> &Self {
        v
    }
}

impl nsIXPCTestInterfaceA {
    /// Cast this `nsIXPCTestInterfaceA` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestInterfaceACoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestInterfaceA {
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
impl<T: nsISupportsCoerce> nsIXPCTestInterfaceACoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestInterfaceA) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestInterfaceA
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestInterfaceAVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* attribute string name; */
    pub GetName: unsafe extern "system" fn (this: *const nsIXPCTestInterfaceA, aName: *mut *const libc::c_char) -> ::nserror::nsresult,

    /* attribute string name; */
    pub SetName: unsafe extern "system" fn (this: *const nsIXPCTestInterfaceA, aName: *const libc::c_char) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestInterfaceA {


    /// `attribute string name;`
    #[inline]
    pub unsafe fn GetName(&self, aName: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).GetName)(self, aName)
    }



    /// `attribute string name;`
    #[inline]
    pub unsafe fn SetName(&self, aName: *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).SetName)(self, aName)
    }


}


/// `interface nsIXPCTestInterfaceB : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestInterfaceB {
    vtable: *const nsIXPCTestInterfaceBVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestInterfaceB.
unsafe impl XpCom for nsIXPCTestInterfaceB {
    const IID: nsIID = nsID(0xff528c3a, 0x2410, 0x46de,
        [0xac, 0xaa, 0x44, 0x9a, 0xa6, 0x40, 0x3a, 0x33]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestInterfaceB {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestInterfaceB.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestInterfaceBCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestInterfaceB`.
    fn coerce_from(v: &nsIXPCTestInterfaceB) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestInterfaceBCoerce for nsIXPCTestInterfaceB {
    #[inline]
    fn coerce_from(v: &nsIXPCTestInterfaceB) -> &Self {
        v
    }
}

impl nsIXPCTestInterfaceB {
    /// Cast this `nsIXPCTestInterfaceB` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestInterfaceBCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestInterfaceB {
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
impl<T: nsISupportsCoerce> nsIXPCTestInterfaceBCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestInterfaceB) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestInterfaceB
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestInterfaceBVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* attribute string name; */
    pub GetName: unsafe extern "system" fn (this: *const nsIXPCTestInterfaceB, aName: *mut *const libc::c_char) -> ::nserror::nsresult,

    /* attribute string name; */
    pub SetName: unsafe extern "system" fn (this: *const nsIXPCTestInterfaceB, aName: *const libc::c_char) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestInterfaceB {


    /// `attribute string name;`
    #[inline]
    pub unsafe fn GetName(&self, aName: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).GetName)(self, aName)
    }



    /// `attribute string name;`
    #[inline]
    pub unsafe fn SetName(&self, aName: *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).SetName)(self, aName)
    }


}


/// `interface nsIXPCTestInterfaceC : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestInterfaceC {
    vtable: *const nsIXPCTestInterfaceCVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestInterfaceC.
unsafe impl XpCom for nsIXPCTestInterfaceC {
    const IID: nsIID = nsID(0x401cf1b4, 0x355b, 0x4cee,
        [0xb7, 0xb3, 0xc7, 0x97, 0x3a, 0xee, 0x49, 0xbd]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestInterfaceC {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestInterfaceC.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestInterfaceCCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestInterfaceC`.
    fn coerce_from(v: &nsIXPCTestInterfaceC) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestInterfaceCCoerce for nsIXPCTestInterfaceC {
    #[inline]
    fn coerce_from(v: &nsIXPCTestInterfaceC) -> &Self {
        v
    }
}

impl nsIXPCTestInterfaceC {
    /// Cast this `nsIXPCTestInterfaceC` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestInterfaceCCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestInterfaceC {
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
impl<T: nsISupportsCoerce> nsIXPCTestInterfaceCCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestInterfaceC) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestInterfaceC
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestInterfaceCVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* attribute long someInteger; */
    pub GetSomeInteger: unsafe extern "system" fn (this: *const nsIXPCTestInterfaceC, aSomeInteger: *mut i32) -> ::nserror::nsresult,

    /* attribute long someInteger; */
    pub SetSomeInteger: unsafe extern "system" fn (this: *const nsIXPCTestInterfaceC, aSomeInteger: i32) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestInterfaceC {


    /// `attribute long someInteger;`
    #[inline]
    pub unsafe fn GetSomeInteger(&self, aSomeInteger: *mut i32) -> ::nserror::nsresult {
        ((*self.vtable).GetSomeInteger)(self, aSomeInteger)
    }



    /// `attribute long someInteger;`
    #[inline]
    pub unsafe fn SetSomeInteger(&self, aSomeInteger: i32) -> ::nserror::nsresult {
        ((*self.vtable).SetSomeInteger)(self, aSomeInteger)
    }


}


