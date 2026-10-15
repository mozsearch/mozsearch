{
  runCommandLocal,
  cmake,
  which,
  wasm-pack,
  wasm-snip,
  llvmPackages,
  binaryen,
  protobuf,
  buildWasmBindgenCli,
  fetchCrate,
  rustPlatform,
  craneLib,
}: let
  wasm-bindgen-cli = buildWasmBindgenCli rec {
    src = fetchCrate {
      pname = "wasm-bindgen-cli";
      version = "0.2.129";
      hash = "sha256-pcecKQd7E8Opw6bkFoE569epUi7gh5qpQF1e5PJY6V8=";
    };

    cargoDeps = rustPlatform.fetchCargoVendor {
      inherit src;
      inherit (src) pname version;
      hash = "sha256-vmUrWVU7kPJJxO5qIVeAkwQyWDELO1Z4Z5gitz2kco8=";
    };
  };
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
