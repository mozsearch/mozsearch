# Citations of web specs in a file without other analysis (see
# tools/src/spec_analyze/citations.rs), whose steps' scopes are by
# indentation.


# https://example.spec.whatwg.org/#assemble
def assemble(widgets):
    # Step 1.
    node = None
    for widget in widgets:
        # Step 2.1. Poke it.
        poke(widget)
    # Step 2.2.
    return node


# Step 3.  (Not in assemble's scope.)
def poke(widget):
    pass
