A crash set's store (see tools/src/crash_sets.rs), for the webtests (see
webtest-config.json's `crash_sets_path`): a set of 6 crashes (plain JSON, as
the store reads too) of a QuotaManager shutdown timeout, whose QuotaManager IO
thread was busy in dom/quota in 3 (2 variants), whose DOMCacheThread was busy
in dom/cache in 2, with nothing else in 1, and whose Link Monitor thread was
busy in all of them.  Its manifest is in sets/, by its ID, which
`crash-set-fetch --manifest` computes.
