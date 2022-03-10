//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/xpcom/base/nsISupports.idl
//


/// `interface nsISupports`
///


// The actual type definition for the interface. This struct has methods
// declared on it which will call through its vtable. You never want to pass
// this type around by value, always pass it behind a reference.

#[repr(C)]
pub struct nsISupports {
    vtable: *const nsISupportsVTable,

    /// This field is a phantomdata to ensure that the VTable type and any
    /// struct containing it is not safe to send across threads, as XPCOM is
    /// generally not threadsafe.
    ///
    /// XPCOM interfaces in general are not safe to send across threads.
    __nosync: ::std::marker::PhantomData<::std::rc::Rc<u8>>,
}

// Implementing XpCom for an interface exposes its IID, which allows for easy
// use of the `.query_interface<T>` helper method. This also defines that
// method for nsISupports.
unsafe impl XpCom for nsISupports {
    const IID: nsIID = nsID(0x00000000, 0x0000, 0x0000,
        [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46]);
}

// We need to implement the RefCounted trait so we can be used with `RefPtr`.
// This trait teaches `RefPtr` how to manage our memory.
unsafe impl RefCounted for nsISupports {
    #[inline]
    unsafe fn addref(&self) {
        self.AddRef();
    }
    #[inline]
    unsafe fn release(&self) {
        self.Release();
    }
}

// This trait is implemented on all types which can be coerced to from nsISupports.
// It is used in the implementation of `fn coerce<T>`. We hide it from the
// documentation, because it clutters it up a lot.
#[doc(hidden)]
pub trait nsISupportsCoerce {
    /// Cheaply cast a value of this type from a `nsISupports`.
    fn coerce_from(v: &nsISupports) -> &Self;
}

// The trivial implementation: We can obviously coerce ourselves to ourselves.
impl nsISupportsCoerce for nsISupports {
    #[inline]
    fn coerce_from(v: &nsISupports) -> &Self {
        v
    }
}

impl nsISupports {
    /// Cast this `nsISupports` to one of its base interfaces.
    #[inline]
    pub fn coerce<T: nsISupportsCoerce>(&self) -> &T {
        T::coerce_from(self)
    }
}

// This struct represents the interface's VTable. A pointer to a statically
// allocated version of this struct is at the beginning of every nsISupports
// object. It contains one pointer field for each method in the interface. In
// the case where we can't generate a binding for a method, we include a void
// pointer.
#[doc(hidden)]
#[repr(C)]
pub struct nsISupportsVTable {/* void QueryInterface (in nsIIDRef uuid, [iid_is (uuid), retval] out nsQIResult result); */
    pub QueryInterface: unsafe extern "system" fn (this: *const nsISupports, uuid: *const nsIID, result: *mut *mut libc::c_void) -> ::nserror::nsresult,

    /* [noscript,notxpcom] nsrefcnt AddRef (); */
    pub AddRef: unsafe extern "system" fn (this: *const nsISupports) -> nsrefcnt,

    /* [noscript,notxpcom] nsrefcnt Release (); */
    pub Release: unsafe extern "system" fn (this: *const nsISupports) -> nsrefcnt,
}


// The implementations of the function wrappers which are exposed to rust code.
// Call these methods rather than manually calling through the VTable struct.
impl nsISupports {


    /// `void QueryInterface (in nsIIDRef uuid, [iid_is (uuid), retval] out nsQIResult result);`
    #[inline]
    pub unsafe fn QueryInterface(&self, uuid: *const nsIID, result: *mut *mut libc::c_void) -> ::nserror::nsresult {
        ((*self.vtable).QueryInterface)(self, uuid, result)
    }



    /// `[noscript,notxpcom] nsrefcnt AddRef ();`
    #[inline]
    pub unsafe fn AddRef(&self, ) -> nsrefcnt {
        ((*self.vtable).AddRef)(self, )
    }



    /// `[noscript,notxpcom] nsrefcnt Release ();`
    #[inline]
    pub unsafe fn Release(&self, ) -> nsrefcnt {
        ((*self.vtable).Release)(self, )
    }


}


