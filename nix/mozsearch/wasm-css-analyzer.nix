{
  runCommandLocal,
  cmake,
  which,
  wasm-pack,
  wasm-snip,
  llvmPackages,
  binaryen,
  protobuf,
  wasm-bindgen-cli_0_2_114,
  craneLib,
  breakpointHook,
}: let
  wasm-bindgen-cli = wasm-bindgen-cli_0_2_114;
  subdir = "scripts/web-analyze/wasm-css-analyzer";

  commonArgs = {
    src = runCommandLocal "mozsearch-wasm-css-analyzer-source" {} ''
      mkdir -p $out
      mkdir -p $out/scripts/web-analyze/
      cp -r ${../../${subdir}} $out/${subdir}
      cp -r ${../../tools} $out/tools
      cp -r ${../../deps} $out/deps
    '';

    sourceRoot = "mozsearch-wasm-css-analyzer-source/${subdir}";
    cargoToml = ../../${subdir}/Cargo.toml;
    cargoLock = ../../${subdir}/Cargo.lock;

    CARGO_BUILD_TARGET = "wasm32-unknown-unknown";

    nativeBuildInputs = [
      cmake
      which
      wasm-pack
      wasm-snip
      binaryen
      wasm-bindgen-cli
      llvmPackages.bintools
      protobuf
      breakpointHook
    ];
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
  craneLib.buildPackage (commonArgs
    // {
      inherit cargoArtifacts;

      cargoBuildCommand = "HOME=$(pwd) ./build.sh";

      installPhaseCommand = ''
        mkdir -p $out/share/wasm-css-analyzer
        install -Dm644 out/* $out/share/wasm-css-analyzer
      '';

      doCheck = false;
    })
