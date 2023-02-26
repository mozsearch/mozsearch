Put symlinks to trees you want to index in here.

The configs for the trees go in the parallel `tree-configs` directory.

You can then build the trees via `make build-trees` from `/vagrant`
inside of your docker image if your config file is the default of
`config.json`.  If your config file is named something else, then
you can set the `CONFIG` env variable by doing something like
`CONFIG=my-config.json make build-trees`.  Note that although make
can accept variable assignments as part of its arguments, that's not
how this is intended to work, and so maybe it won't work!
