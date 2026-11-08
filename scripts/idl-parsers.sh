# (Sourced, by idl-analyze.sh and try-analyze-rest.sh.)
#
# use_idl_parsers FILES_ROOT DIR: the xpidl and WebIDL parsers (with ply) of
# the tree at FILES_ROOT, if it has them, copied into DIR and first in
# PYTHONPATH.  They are unstable and for instance the esr140 and nightly tree
# disagree on how to define async iterables in WebIDL.  Without them, the
# parsers are PYTHONPATH's.
use_idl_parsers() {
    local FILES=$1 DIR=$2
    if [ -f "$FILES/xpcom/idl-parser/xpidl/xpidl.py" ] && [ -f "$FILES/dom/bindings/parser/WebIDL.py" ]; then
        mkdir -p "$DIR/ply"
        cp "$FILES/xpcom/idl-parser/xpidl/xpidl.py" "$FILES/dom/bindings/parser/WebIDL.py" "$DIR/"
        local PLYFILE
        for PLYFILE in __init__.py lex.py yacc.py; do
            cp "$FILES/other-licenses/ply/ply/$PLYFILE" "$DIR/ply/" 2> /dev/null ||
                cp "$FILES/third_party/python/ply/ply/$PLYFILE" "$DIR/ply/"
        done
        export PYTHONPATH="$DIR${PYTHONPATH:+:$PYTHONPATH}"
    fi
}
