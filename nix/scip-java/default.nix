{
  lib,
  makeBinaryWrapper,
  mkSbtDerivation,
  fetchFromGitHub,
  openjdk8,
  openjdk11,
  openjdk17,
  openjdk21,
}: let
  pname = "scip-java";
  version = "0.10.3";

  JAVA8 = "${openjdk8}/lib/openjdk";
  JAVA11 = "${openjdk11}/lib/openjdk";
  JAVA17 = "${openjdk17}/lib/openjdk";
  JAVA21 = "${openjdk21}/lib/openjdk";
in
  mkSbtDerivation {
    inherit pname version;

    nativeBuildInputs = [
      makeBinaryWrapper
    ];

    src = fetchFromGitHub {
      owner = "sourcegraph";
      repo = pname;
      rev = "v${version}";
      hash = "sha256-jDOu0/2di49mkmSpnOiiA/ojD4jbOGOgf6PdxzchtP8=";
    };

    patches = [
      ./0001-Use-system-JVM.patch
    ];

    inherit JAVA8 JAVA11 JAVA17 JAVA21;

    depsWarmupCommand = ''
      export JAVA8="${JAVA8}"
      export JAVA11="${JAVA11}"
      export JAVA17="${JAVA17}"
      export JAVA21="${JAVA21}"

      sbt cli/pack
    '';

    depsSha256 = "sha256-QbE6VXEAteYHU3kFepxKXVGCgjM7BdgK/M8IOrGViw4=";

    buildPhase = ''
      runHook preBuild

      sbt cli/pack

      runHook postBuild
    '';

    installPhase = ''
      runHook preInstall

      mkdir -p $out
      cp -R scip-java/target/pack/lib $out/lib
      cp -R scip-java/target/pack/bin $out/bin
      rm $out/bin/*.bat

      runHook postInstall
    '';

    postFixup = ''
      wrapProgram $out/bin/scip-java --set JAVA_HOME "${JAVA21}"
      wrapProgram $out/bin/bazel-build-tool --set JAVA_HOME "${JAVA21}"
    '';
  }
