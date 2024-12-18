{
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/nixpkgs-unstable;
    flake-utils.url = github:numtide/flake-utils;
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    sbt = {
      url = "github:zaninime/sbt-derivation";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
      };
    };
  };

  nixConfig = {
    extra-substituters = ["https://nix-community.cachix.org"];
    extra-trusted-public-keys = ["nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="];
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    fenix,
    sbt,
  }: (
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system}.extend fenix.overlays.default;

        rustToolchain = pkgs.fenix.combine (with pkgs.fenix; [
          stable.minimalToolchain
          stable.rust-src
          rust-analyzer
          targets.wasm32-unknown-unknown.stable.rust-std
        ]);

        mkSbtDerivation = sbt.mkSbtDerivation.${system};

        pythonPackages = p:
          with p; [
            boto3
            rich
          ];
      in rec {
        packages = {
          scip-python = pkgs.callPackage ./nix/scip-python {};
          scip-typescript = pkgs.callPackage ./nix/scip-typescript {};
          scip-java = pkgs.callPackage ./nix/scip-java {
            inherit mkSbtDerivation;
          };
          wasm-snip = pkgs.callPackage ./nix/wasm-snip {};
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            jq

            (python3.withPackages pythonPackages)
            awscli2

            # Dependencies required to build tools
            rustToolchain
            openssl
            cmake
            pkg-config

            # Must be before (unwrapped) clang in path
            clang-tools_19

            # Dependencies required to build clang-plugin
            clang_19
            llvmPackages_19.libllvm
            llvmPackages_19.libclang

            gdb

            scip
            protobuf

            pre-commit
          ];

          AWS_PROFILE = "searchfox";
        };

        formatter = pkgs.alejandra;
      }
    )
  );
}
