{
  lib,
  stdenv,
  gradle_9,
  makeWrapper,
  jre_headless,
  fetchFromGitHub,
}: let
  jre = jre_headless;
  gradle = gradle_9;
  self = stdenv.mkDerivation (finalAttrs: {
    pname = "scip-java";
    version = "0.13.2";

    src = fetchFromGitHub {
      owner = "scip-code";
      repo = finalAttrs.pname;
      rev = "v${finalAttrs.version}";
      hash = "sha256-DGpEqg/CF1/i2d+L/E0fZ0VTfQ9ir+rjx6tIdb3RWtA=";
    };

    nativeBuildInputs = [
      gradle
      makeWrapper
    ];

    gradleUpdateTask = "--write-verification-metadata sha256";
    gradleBuildTask = ":scip-java:installDist";

    mitmCache = gradle.fetchDeps {
      pkg = self;
      data = ./deps.json;
    };

    gradleFlags = ["-PreleaseVersion=${finalAttrs.version}"];

    installPhase = ''
      runHook preInstall

      install -Dm644 -t $out/share/scip-java scip-java/build/install/scip-java/lib/*.jar

      for jar in $out/share/scip-java/*.jar; do
        classpath=''${classpath-}''${classpath:+:}''${jar}
      done

      makeWrapper ${lib.getExe jre} $out/bin/scip-java \
        --add-flags "-cp $classpath org.scip_code.scip_java.ScipJava"

      runHook postInstall
    '';

    doCheck = false;
  });
in
  self
