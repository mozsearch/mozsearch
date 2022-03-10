//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/xpcom/base/nsISupports.idl
//


{static D: &'static [Interface] = &[

        Interface {
            name: "nsISupports",
            base: None,
            methods: Ok(&[
                    /* void QueryInterface (in nsIIDRef uuid, [iid_is (uuid), retval] out nsQIResult result); */
                    Method {
                        name: "QueryInterface",
                        params: &[Param { name: "uuid", ty: "*const nsIID" }, Param { name: "result", ty: "*mut *mut libc::c_void" }],
                        ret: "::nserror::nsresult",
                    },

                    /* [noscript,notxpcom] nsrefcnt AddRef (); */
                    Method {
                        name: "AddRef",
                        params: &[],
                        ret: "nsrefcnt",
                    },

                    /* [noscript,notxpcom] nsrefcnt Release (); */
                    Method {
                        name: "Release",
                        params: &[],
                        ret: "nsrefcnt",
                    },

                    ]),
        },

        ]; D}

