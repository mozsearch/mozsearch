{
  stdenv,
  gradle,
  jdk21,
}: let
  self = stdenv.mkDerivation (finalAttrs: {
    pname = "test-gradle-project";
    version = "unstable";

    src = ../../../../../tests/tests/files;

    nativeBuildInputs = [
      gradle
      jdk21
    ];

    mitmCache = gradle.fetchDeps {
      pkg = self;
      data = ./deps.json;
    };

    installPhase = ''
      runHook preInstall

      touch $out

      runHook postInstall
    '';
  });
in
  self
