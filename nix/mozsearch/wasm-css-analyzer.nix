{
  rustPlatform,
  runCommandLocal,
  cmake,
  which,
  wasm-pack,
  wasm-snip,
  llvmPackages,
  binaryen,
  wasm-bindgen-cli_0_2_92,
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
      wasm-bindgen-cli_0_2_92
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
