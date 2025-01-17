{
  stdenv,
  lib,
  runCommandLocal,
  makeWrapper,
  python3,
  graphviz,
  spidermonkey_128,
  parallel,
  wget,
  codesearch,
  git,
  envsubst,
  jq,
  webidl,
  xpidl,
  nginx,
  vmtouch,
  gnugrep,
  livegrep-grpc3,
  mozsearch-tools,
  mozsearch-clang-plugin,
  mozsearch-wasm-css-analyzer,
  llvmPackages,
  procps,
}:
stdenv.mkDerivation {
  pname = "mozsearch-scripts";
  version = "unstable";

  src = runCommandLocal "mozsearch-scripts-source" {} ''
    mkdir -p $out
    cp -r ${../../scripts} $out/scripts
    cp -r ${../../infrastructure} $out/infrastructure
    cp -r ${../../sax} $out/sax
    cp -r ${../../config_defaults} $out/config_defaults
    cp -r ${../../router} $out/router
    cp -r ${../../static} $out/static
    cp -r ${../../tests} $out/tests
  '';

  nativeBuildInputs = [
    makeWrapper
  ];

  buildInputs = [
    (python3.withPackages (p: [(webidl p) (xpidl p) (livegrep-grpc3 p)]))
  ];

  installPhase = ''
    runHook preInstall

    mkdir -p $out
    cp -R infrastructure $out/infrastructure
    cp -R scripts $out/scripts
    cp -R sax $out/sax
    cp -R config_defaults $out/config_defaults
    cp -R router $out/router
    cp -r static $out/static
    cp -r tests $out/tests

    runHook postInstall
  '';

  # Specify script's runtime dependencies
  postFixup = ''
    wrapProgram $out/router/router.py --prefix PATH : ${lib.makeBinPath [
      llvmPackages.bintools # for c++filt
    ]}

    wrapProgram $out/infrastructure/indexer-setup.sh --prefix PATH : ${lib.makeBinPath [
      mozsearch-tools
      envsubst
      jq
    ]}

    wrapProgram $out/infrastructure/indexer-run.sh --prefix PATH : ${lib.makeBinPath [
      mozsearch-tools
      graphviz
      spidermonkey_128
      parallel
      wget
      codesearch
      git
      jq
      procps
    ]} \
      --set MOZSEARCH_CLANG_PLUGIN_DIR "${mozsearch-clang-plugin}/lib" \
      --set MOZSEARCH_WASM_DIR "${mozsearch-wasm-css-analyzer}/share/wasm-css-analyzer"

    wrapProgram $out/infrastructure/web-server-setup.sh --prefix PATH : ${lib.makeBinPath [
      envsubst
      jq
      vmtouch
      nginx
      procps
    ]}

    wrapProgram $out/infrastructure/web-server-run.sh --prefix PATH : ${lib.makeBinPath [
      procps
      codesearch
      mozsearch-tools
      gnugrep
      graphviz
    ]}
  '';
}
