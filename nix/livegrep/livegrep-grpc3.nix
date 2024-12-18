{fetchFromGitHub}: let
  source = import ./src.nix {inherit fetchFromGitHub;};
in
  {
    buildPythonPackage,
    grpcio-tools,
    grpcio,
    python,
    ...
  }:
    buildPythonPackage {
      pname = "livegrep-grpc3";
      inherit (source) version src;

      format = "other";

      nativeBuildInputs = [
        grpcio-tools
      ];

      dependencies = [
        grpcio
      ];

      postPatch = ''
        sed 's|import "src/proto/config.proto";|import "livegrep/config.proto";|' -i src/proto/livegrep.proto
      '';

      buildPhase = ''
        runHook postBuild

        mkdir build
        python3 -m grpc_tools.protoc --python_out=build --grpc_python_out=build -I livegrep=src/proto "src/proto/config.proto" "src/proto/livegrep.proto"
        touch build/livegrep/__init__.py

        runHook postBuild
      '';

      installPhase = ''
        runHook preInstall

        mkdir -p "$out/lib/python${python.pythonVersion}/site-packages/"
        mv build/livegrep "$out/lib/python${python.pythonVersion}/site-packages/livegrep"

        runHook postInstall
      '';
    }
