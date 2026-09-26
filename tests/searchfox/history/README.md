# History configuration for the searchfox tree

This directory is passed to `build-syntax-token-tree` by `../setup`.  Each
`revs/<rev>.toml` file decorates the source revision it's named after; its
`attributes` (in `.gitattributes` syntax) apply to that revision and all of
its descendants until another note provides attributes.  See
`tools/src/hyperblame/history_config.rs` for details.

Changing a note that applies to already processed revisions requires
regenerating the history from the earliest affected revision;
`build-syntax-token-tree` refuses to run until then.
