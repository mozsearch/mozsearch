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
  };
  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;

      # Some tests write history notes with git fast-import.
      nativeCheckInputs = [
        gitMinimal
      ];

      postFixup = ''
        wrapProgram $out/bin/pipeline-server \
          --prefix PATH : ${lib.makeBinPath [graphviz]}
      '';
    })
