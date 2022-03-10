//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_interfaces.idl
//


{static D: &'static [Interface] = &[

        Interface {
            name: "nsIXPCTestInterfaceA",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* attribute string name; */
                    Method {
                        name: "GetName",
                        params: &[Param { name: "aName", ty: "*mut *const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetName",
                        params: &[Param { name: "aName", ty: "*const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        Interface {
            name: "nsIXPCTestInterfaceB",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* attribute string name; */
                    Method {
                        name: "GetName",
                        params: &[Param { name: "aName", ty: "*mut *const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetName",
                        params: &[Param { name: "aName", ty: "*const libc::c_char" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        Interface {
            name: "nsIXPCTestInterfaceC",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* attribute long someInteger; */
                    Method {
                        name: "GetSomeInteger",
                        params: &[Param { name: "aSomeInteger", ty: "*mut i32" }],
                        ret: "::nserror::nsresult",
                    },
                    Method {
                        name: "SetSomeInteger",
                        params: &[Param { name: "aSomeInteger", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        ]; D}

