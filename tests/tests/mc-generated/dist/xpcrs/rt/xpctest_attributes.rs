//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_attributes.idl
//


/// `interface nsIXPCTestObjectReadOnly : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestObjectReadOnly {
    vtable: *const nsIXPCTestObjectReadOnlyVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestObjectReadOnly.
unsafe impl XpCom for nsIXPCTestObjectReadOnly {
    const IID: nsIID = nsID(0x42fbd9f6, 0xb12d, 0x47ef,
        [0xb7, 0xa1, 0x02, 0xd7, 0x3c, 0x11, 0xfe, 0x53]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestObjectReadOnly {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestObjectReadOnly.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestObjectReadOnlyCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestObjectReadOnly`.
    fn coerce_from(v: &nsIXPCTestObjectReadOnly) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestObjectReadOnlyCoerce for nsIXPCTestObjectReadOnly {
    #[inline]
    fn coerce_from(v: &nsIXPCTestObjectReadOnly) -> &Self {
        v
    }
}

impl nsIXPCTestObjectReadOnly {
    /// Cast this `nsIXPCTestObjectReadOnly` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestObjectReadOnlyCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestObjectReadOnly {
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
impl<T: nsISupportsCoerce> nsIXPCTestObjectReadOnlyCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestObjectReadOnly) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestObjectReadOnly
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestObjectReadOnlyVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* readonly attribute string strReadOnly; */
    pub GetStrReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aStrReadOnly: *mut *const libc::c_char) -> ::nserror::nsresult,

    /* readonly attribute boolean boolReadOnly; */
    pub GetBoolReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aBoolReadOnly: *mut bool) -> ::nserror::nsresult,

    /* readonly attribute short shortReadOnly; */
    pub GetShortReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aShortReadOnly: *mut i16) -> ::nserror::nsresult,

    /* readonly attribute long longReadOnly; */
    pub GetLongReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aLongReadOnly: *mut i32) -> ::nserror::nsresult,

    /* readonly attribute float floatReadOnly; */
    pub GetFloatReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aFloatReadOnly: *mut libc::c_float) -> ::nserror::nsresult,

    /* readonly attribute char charReadOnly; */
    pub GetCharReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aCharReadOnly: *mut libc::c_char) -> ::nserror::nsresult,

    /* readonly attribute PRTime timeReadOnly; */
    pub GetTimeReadOnly: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadOnly, aTimeReadOnly: *mut PRTime) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestObjectReadOnly {


    /// `readonly attribute string strReadOnly;`
    #[inline]
    pub unsafe fn GetStrReadOnly(&self, aStrReadOnly: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).GetStrReadOnly)(self, aStrReadOnly)
    }



    /// `readonly attribute boolean boolReadOnly;`
    #[inline]
    pub unsafe fn GetBoolReadOnly(&self, aBoolReadOnly: *mut bool) -> ::nserror::nsresult {
        ((*self.vtable).GetBoolReadOnly)(self, aBoolReadOnly)
    }



    /// `readonly attribute short shortReadOnly;`
    #[inline]
    pub unsafe fn GetShortReadOnly(&self, aShortReadOnly: *mut i16) -> ::nserror::nsresult {
        ((*self.vtable).GetShortReadOnly)(self, aShortReadOnly)
    }



    /// `readonly attribute long longReadOnly;`
    #[inline]
    pub unsafe fn GetLongReadOnly(&self, aLongReadOnly: *mut i32) -> ::nserror::nsresult {
        ((*self.vtable).GetLongReadOnly)(self, aLongReadOnly)
    }



    /// `readonly attribute float floatReadOnly;`
    #[inline]
    pub unsafe fn GetFloatReadOnly(&self, aFloatReadOnly: *mut libc::c_float) -> ::nserror::nsresult {
        ((*self.vtable).GetFloatReadOnly)(self, aFloatReadOnly)
    }



    /// `readonly attribute char charReadOnly;`
    #[inline]
    pub unsafe fn GetCharReadOnly(&self, aCharReadOnly: *mut libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).GetCharReadOnly)(self, aCharReadOnly)
    }



    /// `readonly attribute PRTime timeReadOnly;`
    #[inline]
    pub unsafe fn GetTimeReadOnly(&self, aTimeReadOnly: *mut PRTime) -> ::nserror::nsresult {
        ((*self.vtable).GetTimeReadOnly)(self, aTimeReadOnly)
    }


}


/// `interface nsIXPCTestObjectReadWrite : nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsIXPCTestObjectReadWrite {
    vtable: *const nsIXPCTestObjectReadWriteVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsIXPCTestObjectReadWrite.
unsafe impl XpCom for nsIXPCTestObjectReadWrite {
    const IID: nsIID = nsID(0xf07529b0, 0xa479, 0x4954,
        [0xab, 0xa5, 0xab, 0x31, 0x42, 0xc6, 0xb1, 0xcb]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsIXPCTestObjectReadWrite {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsIXPCTestObjectReadWrite.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsIXPCTestObjectReadWriteCoerce {
    /// Cheaply cast a value of this type from a `nsIXPCTestObjectReadWrite`.
    fn coerce_from(v: &nsIXPCTestObjectReadWrite) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsIXPCTestObjectReadWriteCoerce for nsIXPCTestObjectReadWrite {
    #[inline]
    fn coerce_from(v: &nsIXPCTestObjectReadWrite) -> &Self {
        v
    }
}

impl nsIXPCTestObjectReadWrite {
    /// Cast this `nsIXPCTestObjectReadWrite` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsIXPCTestObjectReadWriteCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// Every interface struct type implements `Deref` to its base interface. This
// causes methods on the base interfaces to be directly avaliable on the
// object. For example, you can call `.AddRef` or `.QueryInterface` directly
// on any interface which inherits from `nsISupports`.
impl ::std::ops::Deref for nsIXPCTestObjectReadWrite {
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
impl<T: nsISupportsCoerce> nsIXPCTestObjectReadWriteCoerce for T {
    #[inline]
    fn coerce_from(v: &nsIXPCTestObjectReadWrite) -> &Self {
        T::coerce_from(v)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsIXPCTestObjectReadWrite
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsIXPCTestObjectReadWriteVTable {
    /// We need to include the members from the base interface's vtable at the start
    /// of the VTable definition.
    pub __base: nsISupportsVTable,

    /* attribute string stringProperty; */
    pub GetStringProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aStringProperty: *mut *const libc::c_char) -> ::nserror::nsresult,

    /* attribute string stringProperty; */
    pub SetStringProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aStringProperty: *const libc::c_char) -> ::nserror::nsresult,

    /* attribute boolean booleanProperty; */
    pub GetBooleanProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aBooleanProperty: *mut bool) -> ::nserror::nsresult,

    /* attribute boolean booleanProperty; */
    pub SetBooleanProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aBooleanProperty: bool) -> ::nserror::nsresult,

    /* attribute short shortProperty; */
    pub GetShortProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aShortProperty: *mut i16) -> ::nserror::nsresult,

    /* attribute short shortProperty; */
    pub SetShortProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aShortProperty: i16) -> ::nserror::nsresult,

    /* attribute long longProperty; */
    pub GetLongProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aLongProperty: *mut i32) -> ::nserror::nsresult,

    /* attribute long longProperty; */
    pub SetLongProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aLongProperty: i32) -> ::nserror::nsresult,

    /* attribute float floatProperty; */
    pub GetFloatProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aFloatProperty: *mut libc::c_float) -> ::nserror::nsresult,

    /* attribute float floatProperty; */
    pub SetFloatProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aFloatProperty: libc::c_float) -> ::nserror::nsresult,

    /* attribute char charProperty; */
    pub GetCharProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aCharProperty: *mut libc::c_char) -> ::nserror::nsresult,

    /* attribute char charProperty; */
    pub SetCharProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aCharProperty: libc::c_char) -> ::nserror::nsresult,

    /* attribute PRTime timeProperty; */
    pub GetTimeProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aTimeProperty: *mut PRTime) -> ::nserror::nsresult,

    /* attribute PRTime timeProperty; */
    pub SetTimeProperty: unsafe extern "system" fn (this: *const nsIXPCTestObjectReadWrite, aTimeProperty: PRTime) -> ::nserror::nsresult,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsIXPCTestObjectReadWrite {


    /// `attribute string stringProperty;`
    #[inline]
    pub unsafe fn GetStringProperty(&self, aStringProperty: *mut *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).GetStringProperty)(self, aStringProperty)
    }



    /// `attribute string stringProperty;`
    #[inline]
    pub unsafe fn SetStringProperty(&self, aStringProperty: *const libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).SetStringProperty)(self, aStringProperty)
    }



    /// `attribute boolean booleanProperty;`
    #[inline]
    pub unsafe fn GetBooleanProperty(&self, aBooleanProperty: *mut bool) -> ::nserror::nsresult {
        ((*self.vtable).GetBooleanProperty)(self, aBooleanProperty)
    }



    /// `attribute boolean booleanProperty;`
    #[inline]
    pub unsafe fn SetBooleanProperty(&self, aBooleanProperty: bool) -> ::nserror::nsresult {
        ((*self.vtable).SetBooleanProperty)(self, aBooleanProperty)
    }



    /// `attribute short shortProperty;`
    #[inline]
    pub unsafe fn GetShortProperty(&self, aShortProperty: *mut i16) -> ::nserror::nsresult {
        ((*self.vtable).GetShortProperty)(self, aShortProperty)
    }



    /// `attribute short shortProperty;`
    #[inline]
    pub unsafe fn SetShortProperty(&self, aShortProperty: i16) -> ::nserror::nsresult {
        ((*self.vtable).SetShortProperty)(self, aShortProperty)
    }



    /// `attribute long longProperty;`
    #[inline]
    pub unsafe fn GetLongProperty(&self, aLongProperty: *mut i32) -> ::nserror::nsresult {
        ((*self.vtable).GetLongProperty)(self, aLongProperty)
    }



    /// `attribute long longProperty;`
    #[inline]
    pub unsafe fn SetLongProperty(&self, aLongProperty: i32) -> ::nserror::nsresult {
        ((*self.vtable).SetLongProperty)(self, aLongProperty)
    }



    /// `attribute float floatProperty;`
    #[inline]
    pub unsafe fn GetFloatProperty(&self, aFloatProperty: *mut libc::c_float) -> ::nserror::nsresult {
        ((*self.vtable).GetFloatProperty)(self, aFloatProperty)
    }



    /// `attribute float floatProperty;`
    #[inline]
    pub unsafe fn SetFloatProperty(&self, aFloatProperty: libc::c_float) -> ::nserror::nsresult {
        ((*self.vtable).SetFloatProperty)(self, aFloatProperty)
    }



    /// `attribute char charProperty;`
    #[inline]
    pub unsafe fn GetCharProperty(&self, aCharProperty: *mut libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).GetCharProperty)(self, aCharProperty)
    }



    /// `attribute char charProperty;`
    #[inline]
    pub unsafe fn SetCharProperty(&self, aCharProperty: libc::c_char) -> ::nserror::nsresult {
        ((*self.vtable).SetCharProperty)(self, aCharProperty)
    }



    /// `attribute PRTime timeProperty;`
    #[inline]
    pub unsafe fn GetTimeProperty(&self, aTimeProperty: *mut PRTime) -> ::nserror::nsresult {
        ((*self.vtable).GetTimeProperty)(self, aTimeProperty)
    }



    /// `attribute PRTime timeProperty;`
    #[inline]
    pub unsafe fn SetTimeProperty(&self, aTimeProperty: PRTime) -> ::nserror::nsresult {
        ((*self.vtable).SetTimeProperty)(self, aTimeProperty)
    }


}


