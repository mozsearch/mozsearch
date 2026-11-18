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

  # The tools run git fast-import from this git; see `fast_import_git` in
  # tools/src/git_ops.rs.
  MOZSEARCH_GIT = "${mozsearch-git}/bin/git";
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts MOZSEARCH_GIT;

      # The unit tests are a derivation of their own (`tests`, the flake's
      # mozsearch-tools-tests), so that installing the tools (ex: an indexer's
      # provisioning, or the container's suite) doesn't wait for them: the
      # container's suite builds them while it builds its index, and CI in a
      # step of their own.  (They took about 3.5 of the tools' 6.5 minutes:
      # compiling them 2, and running them 1.5.)
      doCheck = false;

      passthru.tests = craneLib.cargoTest (commonArgs
        // {
          inherit cargoArtifacts MOZSEARCH_GIT;

          # Some tests write history notes with git fast-import, and read them
          # with git.
          nativeCheckInputs = [
            gitMinimal
          ];
        });

      postFixup = ''
        wrapProgram $out/bin/pipeline-server \
          --prefix PATH : ${lib.makeBinPath [graphviz]}
      '';
    })
