{
  rustPlatform,
  runCommandLocal,
  cmake,
  which,
  wasm-pack,
  wasm-snip,
  llvmPackages,
  binaryen,
  wasm-bindgen-cli,
}: let
  subdir = "scripts/web-analyze/wasm-css-analyzer";
in
  rustPlatform.buildRustPackage {
    pname = "mozsearch-wasm-css-analyzer";
    version = "unstable";

    src = runCommandLocal "mozsearch-tools-source" {} ''
      mkdir -p $out
      mkdir -p $out/scripts/web-analyze/
      cp -r ${../../scripts/web-analyze/wasm-css-analyzer} $out/scripts/web-analyze/wasm-css-analyzer
      cp -r ${../../tools} $out/tools
      cp -r ${../../deps} $out/deps
    '';

    buildAndTestSubdir = subdir;
    cargoRoot = subdir;

    cargoLock.lockFile = ../../${subdir}/Cargo.lock;

    CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_LINKER = "lld";

    nativeBuildInputs = [
      cmake
      which
      wasm-pack
      wasm-snip
      binaryen
      (wasm-bindgen-cli.override {
        version = "0.2.92";
        hash = "sha256-1VwY8vQy7soKEgbki4LD+v259751kKxSxmo/gqE6yV0=";
        cargoHash = "sha256-aACJ+lYNEU8FFBs158G1/JG8sc6Rq080PeKCMnwdpH0=";
      })
      llvmPackages.bintools
    ];

    buildPhase = ''
      runHook preBuild

      pushd ${subdir}
      HOME=$(pwd) ./build.sh
      popd

      runHook postBuild
    '';

    installPhase = ''
      runHook preBuild

      mkdir -p $out/share/wasm-css-analyzer
      install -Dm644 ${subdir}/out/* $out/share/wasm-css-analyzer

      runHook postBuild
    '';

    doCheck = false;
  }
