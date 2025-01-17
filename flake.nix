{
  inputs = {
    self.submodules = true;
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
    # Set this using `--override-input self-with-dotgit path:$(pwd)` to provide access to our own repo, required for building the searchfox index itself.
    # This is required because Nix does not copy the whole Flake Git repository to the store, since the git repository format is unstable.
    # See https://github.com/NixOS/nix/issues/6900
    self-with-dotgit = {
      url = "file+file:///dev/null";
      flake = false;
    };
  };

  nixConfig = {
    extra-substituters = ["https://nix-community.cachix.org"];
    extra-trusted-public-keys = ["nix-community.cachix.org-1:mB9FSh9qf2dCimDSUo8Zy7bkq5CX+/rkCWyvRCYg3Fs="];
  };

  outputs = {
    self,
    self-with-dotgit,
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

        rustPlatform = pkgs.makeRustPlatform {
          cargo = rustToolchain;
          rustc = rustToolchain;
        };

        mkSbtDerivation = sbt.mkSbtDerivation.${system};

        llvmPackages = pkgs.llvmPackages_18;

        livegrep-grpc3 = pkgs.callPackage ./nix/livegrep/livegrep-grpc3.nix {};
        webidl = pkgs.callPackage ./nix/idl/webidl.nix {};
        xpidl = pkgs.callPackage ./nix/idl/xpidl.nix {};

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
          codesearch = pkgs.callPackage ./nix/livegrep/codesearch.nix {};
          wasm-snip = pkgs.callPackage ./nix/wasm-snip {};

          mozsearch-tools = pkgs.callPackage ./nix/mozsearch/tools.nix {inherit rustPlatform;};
          mozsearch-clang-plugin = pkgs.callPackage ./nix/mozsearch/clang-plugin.nix {inherit llvmPackages;};
          mozsearch-wasm-css-analyzer = pkgs.callPackage ./nix/mozsearch/wasm-css-analyzer.nix {
            inherit rustPlatform;
            inherit (packages) wasm-snip;
          };
          mozsearch-scripts = pkgs.callPackage ./nix/mozsearch/scripts.nix {
            inherit webidl xpidl livegrep-grpc3;
            inherit (packages) mozsearch-tools mozsearch-clang-plugin mozsearch-wasm-css-analyzer codesearch;
          };

          build-index = pkgs.callPackage ./nix/mozsearch/build-index.nix {
            inherit (packages) mozsearch-scripts;
          };
          serve-index = pkgs.callPackage ./nix/mozsearch/serve-index.nix {
            inherit (packages) mozsearch-scripts;
          };

          tests = pkgs.callPackage ./nix/mozsearch/configs/tests {
            inherit rustToolchain;
            inherit (packages) scip-python scip-java build-index mozsearch-tools mozsearch-clang-plugin serve-index;
          };

          searchfox = pkgs.callPackage ./nix/mozsearch/configs/searchfox.nix {
            inherit self-with-dotgit rustToolchain;
            inherit (packages) build-index mozsearch-tools;
          };
        };

        devShells.default =
          pkgs.mkShell.override {
            stdenv = llvmPackages.stdenv;
          } {
            packages = with pkgs; [
              jq

              (python3.withPackages pythonPackages)
              awscli2

              gdb

              scip
              protobuf

              pre-commit

              llvmPackages.clang-tools
            ];

            inputsFrom = [
              packages.mozsearch-tools
              packages.mozsearch-clang-plugin
              packages.mozsearch-wasm-css-analyzer
            ];

            AWS_PROFILE = "searchfox";
          };

        formatter = pkgs.alejandra;
      }
    )
  );
}
