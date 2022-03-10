//
// DO NOT EDIT.  THIS FILE IS GENERATED FROM $SRCDIR/js/xpconnect/tests/idl/xpctest_returncode.idl
//


{static D: &'static [Interface] = &[

        Interface {
            name: "nsIXPCTestReturnCodeParent",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* nsresult callChild (in long childBehavior); */
                    Method {
                        name: "CallChild",
                        params: &[Param { name: "childBehavior", ty: "i32" }, Param { name: "_retval", ty: "*mut ::nserror::nsresult" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        Interface {
            name: "nsIXPCTestReturnCodeChild",
            base: Some("nsISupports"),
            methods: Ok(&[
                    /* void doIt (in long behavior); */
                    Method {
                        name: "DoIt",
                        params: &[Param { name: "behavior", ty: "i32" }],
                        ret: "::nserror::nsresult",
                    },

                    ]),
        },

        ]; D}

