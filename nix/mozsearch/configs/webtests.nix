{
  self-with-dotgit,
  callPackage,
  scip-python,
  gradle,
  scip-java,
  jdk21,
  rustToolchain,
  rsync,
  jq,
  git,
  git-cinnabar,
  build-index,
  makeWrapper,
  mozsearch-tools,
  mozsearch-clang-plugin,
  mozsearch-scripts,
  serve-index,
  procps,
}: let
  clangStdenv = mozsearch-clang-plugin.passthru.llvmPackages.stdenv;
  test-gradle-project = callPackage tests/test-gradle-project {};
in
  clangStdenv.mkDerivation (finalAttrs: {
    pname = "webtest-index";
    version = "unstable";

    src = self-with-dotgit;

    postPatch = ''
      patchShebangs tests/tests/build tests/tests/setup tests/searchfox/build tests/searchfox/setup
      git add tests/tests/build tests/tests/setup tests/searchfox/build tests/searchfox/setup
      git -c user.name=Nix -c user.email=nix@localhost commit -m "Patch shebangs" --no-verify
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
      mozsearch-tools

      git
      git-cinnabar
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

      export MOZSEARCH_SOURCE_PATH=$(pwd)

      export HOME=$(mktemp -d)
      build-index tests webtest-config.json index

      runHook postBuild
    '';

    dontFixup = true;

    doCheck = true;
    passthru = {
      unchecked = finalAttrs.finalPackage.overrideAttrs (_: {
        doCheck = false;
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

      serve-index tests webtest-config.json index srv

      export SEARCHFOX_SERVER=http://localhost:16995/
      export SEARCHFOX_TREE=tests
      export INSTA_WORKSPACE_ROOT=$(pwd)/tests/tests/checks
      test-index "$INSTA_WORKSPACE_ROOT"

      ${mozsearch-scripts}/scripts/webtest.sh

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
