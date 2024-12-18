{
  buildBazelPackage,
  buildFHSEnv,
  fetchFromGitHub,
  git,
  bazel_5,
  bash,
  coreutils,
}: let
  source = import ./src.nix {inherit fetchFromGitHub;};
in
  buildBazelPackage {
    pname = "livegrep";
    inherit (source) version src;

    postPatch = ''
      rm -f .bazelversion

      # Don't build the web parts, which use rules_js, which writes and calls a #!/usr/bin/env bash script
      rm -rf web
      sed '132,161d' -i WORKSPACE
    '';

    bazel = bazel_5;
    bazelBuildFlags = ["-c opt"];
    bazelTargets = ["//src/tools:codesearch"];

    removeRulesCC = false;

    fetchAttrs = {
      hash = "sha256-zkXgaVSTbAR527rj1/teGMguQ+ok5wJGa5qjFzHjZqM=";
      nativeBuildInputs = [
        git
      ];
    };

    buildAttrs = {
      installPhase = ''
        runHook preInstall

        install -D --strip "bazel-bin/src/tools/codesearch" "$out/bin/codesearch"

        runHook postInstall
      '';
    };
  }
