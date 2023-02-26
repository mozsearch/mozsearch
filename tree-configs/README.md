Put config files for the trees you symlinked in `/trees` in here.  The
assumed default config (which can list multiple trees) is `config.json`.

A good starting point is probably `tests/searchfox-config.json`, noting
that you probably want to increment the `codesearch_port` to start at
port 8082 and keep incrementing from there.  The `tests` repo uses port
8080 and `searchfox` uses 8081, so this avoids edge cases if you are
switching between what is indexed.

You can then build the trees via `make build-trees` from `/vagrant`
inside of your docker image if your config file is the default of
`config.json`.  If your config file is named something else, then
you can set the `CONFIG` env variable by doing something like
`CONFIG=my-config.json make build-trees`.  Note that although make
can accept variable assignments as part of its arguments, that's not
how this is intended to work, and so maybe it won't work!
