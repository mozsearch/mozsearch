# History configuration for the searchfox tree

This directory is passed to `build-syntax-token-tree` by `../setup`.  Each
`revs/<rev>.toml` file decorates the source revision it's named after; its
`attributes` (in `.gitattributes` syntax) apply to that revision and all of
its descendants until another note provides attributes.  An optional
`config.toml` can provide settings for the whole history, like a `start`
revision so that history is only derived for a window of the source history
(this tree doesn't use one).  See `tools/src/hyperblame/history_config.rs` for
details.

Changing a note that applies to already processed revisions (or the start
revision) requires regenerating the history from the earliest affected
revision; `build-syntax-token-tree` refuses to run until then.
