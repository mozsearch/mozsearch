{
  callPackage,
  scip-python,
  gradle,
  scip-java,
  jdk21,
  rustToolchain,
  rsync,
  jq,
  build-index,
  makeWrapper,
  mozsearch-tools,
  mozsearch-clang-plugin,
  serve-index,
  procps,
}: let
  clangStdenv = mozsearch-clang-plugin.passthru.llvmPackages.stdenv;
  test-gradle-project = callPackage ./test-gradle-project {};
in
  clangStdenv.mkDerivation (finalAttrs: {
    pname = "test-repo";
    version = "unstable";

    src = ../../../../tests;

    postPatch = ''
      patchShebangs tests/build tests/setup
    '';

    nativeBuildInputs = [
      # For Python tests
      scip-python

      # For Java tests
      gradle
      scip-java
      jdk21

      # For Rust tests
      rustToolchain

      # For tests setup
      rsync

      # web-server-check.sh deps
      jq

      build-index
      makeWrapper
    ];

    # To update the mitmCache Gradle lock file located at nix/mozsearch/configs/tests/test-gradle-project/deps.json, run:
    # nix build .#configs.tests.mitmCache.updateScript
    # ./result
    mitmCache = test-gradle-project.mitmCache;

    buildPhase = ''
      runHook prebuild

      export LC_ALL=C.UTF-8

      mkdir -p gradle-wrapper
      flags="''${gradleFlagsArray[@]}"
      makeWrapper ${gradle}/bin/gradle gradle-wrapper/gradle --inherit-argv0 --add-flags "$flags"
      PATH="$(pwd)/gradle-wrapper:$PATH"

      build-index . config.json index

      runHook postBuild
    '';

    dontFixup = true;

    doCheck = true;
    passthru = rec {
      unchecked = finalAttrs.finalPackage.overrideAttrs (_: {
        doCheck = false;
      });
      diffable = unchecked.overrideAttrs (_: {
        MOZSEARCH_DIFFABLE = 1;
      });
    };

    nativeCheckInputs = [
      mozsearch-tools
      serve-index
      procps
    ];

    checkPhase = ''
      runHook preCheck

      unset http_proxy
      unset https_proxy

      serve-index . config.json index srv

      export SEARCHFOX_SERVER=http://localhost:16995/
      export SEARCHFOX_TREE=tests
      export INSTA_WORKSPACE_ROOT=$(pwd)/tests/checks
      test-index "$INSTA_WORKSPACE_ROOT"

      # When running in Podman (and probably Docker) Nix seems to wait for all child processes to exit.
      pkill -x nginx

      runHook postCheck
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out
      cp -RL index $out
      rm $out/index/config.json

      runHook postInstall
    '';
  })
