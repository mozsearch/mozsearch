# rust-analyzer from mozsearch's fork (github:mozsearch/rust-analyzer's
# hyperblame branch, the flake's `rust-analyzer-src` input): upstream
# rust-analyzer with the patches we'd upstream, ex: `scip` on several threads,
# and `scip --only-files` for indexing a change's files; for the trees whose
# Rust `rust-analyzer scip` indexes (ex: tests/tests/build).
{
  craneLib,
  rust-analyzer-src,
}: let
  commonArgs = {
    pname = "rust-analyzer";
    version = "0.0.0-mozsearch";
    src = rust-analyzer-src;
    cargoExtraArgs = "-p rust-analyzer --bin rust-analyzer";
    # (As tools.nix's: no debug info to strip.)
    CARGO_PROFILE_RELEASE_DEBUG = "0";
    # (`rust-analyzer --version` says which revision of the fork it is; its
    # build script can't ask git, without the checkout's .git.)
    CFG_RELEASE = "mozsearch-${rust-analyzer-src.shortRev or "dirty"}";
    doCheck = false;
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;
    })
