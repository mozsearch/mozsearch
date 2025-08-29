{
  rustPlatform,
  runCommandLocal,
  pkgconf,
  cmake,
  openssl,
}: let
  subdir = "tools";
in
  rustPlatform.buildRustPackage {
    pname = "mozsearch-tools";
    version = "unstable";

    src = runCommandLocal "mozsearch-tools-source" {} ''
      mkdir -p $out
      cp -r ${../../tools} $out/tools
      cp -r ${../../deps} $out/deps
    '';

    buildAndTestSubdir = subdir;
    cargoRoot = subdir;

    cargoLock = {
      lockFile = ../../${subdir}/Cargo.lock;
      outputHashes = {
        "traitobject-0.1.0" = "sha256-hnCMcSLblsa0BTRxV/Q7wE1C2KByIf51jYvZGoKFV4M=";
      };
    };

    nativeBuildInputs = [
      pkgconf
      cmake
    ];

    buildInputs = [
      openssl
    ];
  }
