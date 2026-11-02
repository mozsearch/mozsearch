A stand-in for crash-stats' API (see tools/src/crash_sets.rs's `Api`), for the
webtests' crash sets' ingestions (see webtest-config.json's `crash_stats_api`):
`SuperSearch.json` answers any search (the tests/crash-reports/ fixtures'
crashes), `SuperSearch.TOKEN.json` a token's (one which can see protected
data, which the ingestion refuses), and `ProcessedCrash/ID.json` is the crash
ID's processed crash (links to tests/crash-reports/).
