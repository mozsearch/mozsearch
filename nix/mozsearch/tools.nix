{
  lib,
  runCommandLocal,
  pkgconf,
  openssl,
  protobuf,
  craneLib,
  makeBinaryWrapper,
  graphviz,
  gitMinimal,
  mozsearch-git,
}: let
  commonArgs = {
    src = runCommandLocal "mozsearch-tools-source" {} ''
      mkdir -p $out
      cp -r ${../../tools} $out/tools
      cp -r ${../../deps} $out/deps
    '';

    nativeBuildInputs = [
      pkgconf
      protobuf
      makeBinaryWrapper
    ];

    buildInputs = [
      openssl
    ];

    sourceRoot = "mozsearch-tools-source/tools";
    cargoToml = ../../tools/Cargo.toml;
    cargoLock = ../../tools/Cargo.lock;

    # The release profile's debug info (for profiling local builds) is
    # stripped from the installed binaries anyway, and before that, crane's
    # removal of references to the vendored sources has to sed through it: on
    # AWS, that took over an hour for the ~3 GB of binaries with debug info.
    CARGO_PROFILE_RELEASE_DEBUG = "0";
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;

      # The tools run git fast-import from this git; see `fast_import_git` in
      # tools/src/git_ops.rs.
      MOZSEARCH_GIT = "${mozsearch-git}/bin/git";

      # Some tests write history notes with git fast-import, and read them
      # with git.
      nativeCheckInputs = [
        gitMinimal
      ];

      postFixup = ''
        wrapProgram $out/bin/pipeline-server \
          --prefix PATH : ${lib.makeBinPath [graphviz]}
      '';
    })
