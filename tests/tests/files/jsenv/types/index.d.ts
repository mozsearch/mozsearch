/// <reference no-default-lib="true" />

// The tests tree's DOM, a lib replacement (@typescript/lib-dom, a package of
// this directory; see ../../package.json), as Firefox's is Gecko's typings
// (tools/@types), generated from its WebIDL, whose members are bound to it.

interface Gadget {
    /** <!-- binding_to(idl, method, WEBIDL_Gadget_poke) --> */
    poke(): void;
}

declare var Gadget: {
    prototype: Gadget;
    new(): Gadget;
};
