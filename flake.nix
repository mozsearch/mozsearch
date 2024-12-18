{
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/nixpkgs-unstable;
    flake-utils.url = github:numtide/flake-utils;
  };

  nixConfig = {
    extra-substituters = [
      "https://searchfox-binary-cache.s3.amazonaws.com?priority=42"
    ];
    extra-trusted-public-keys = [
      "searchfox-binary-cache-1:X2B8qJE4uQJpf42POhKaKf23nlXj+SjifH4OjK7Kgh0="
    ];
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }: (
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
        };

        # A symlink from `docker` to `podman`, because the scripts call `docker`.
        dockerCompat = pkgs.runCommand "docker-podman-compat" {} ''
          mkdir -p $out/bin
          ln -s ${pkgs.podman}/bin/podman $out/bin/docker
        '';

        pythonPackages = p:
          with p; [
            boto3
            rich
          ];

        scip-python = pkgs.callPackage ./nix/scip-python {};

        wasm-snip = pkgs.callPackage ./nix/wasm-snip {};

        commonPackages = with pkgs; [
          livegrep
        ];

        indexerPackages = with pkgs; [
          rust-analyzer
          scip-python
        ];

        serverPackages = with pkgs; [
        ];
      in {
        packages = {
          inherit scip-python wasm-snip;

          indexerPackages = pkgs.symlinkJoin {
            name = "indexerPackages";
            paths = commonPackages ++ indexerPackages;
          };

          serverPackages = pkgs.symlinkJoin {
            name = "serverPackages";
            paths = commonPackages ++ serverPackages;
          };
        };

        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            dockerCompat
            podman

            # Those are probably not all required, copied from
            # https://gist.github.com/adisbladis/187204cb772800489ee3dac4acdd9947
            runc # Container runtime
            conmon # Container runtime monitor
            skopeo # Interact with container registry
            slirp4netns # User-mode networking for unprivileged namespaces
            fuse-overlayfs # CoW for images, much faster than default vfs

            jq

            (python3.withPackages pythonPackages)
            awscli2

            # Dependencies required to build tools
            rustc
            cargo
            rust-analyzer
            rustfmt
            openssl
            cmake
            pkg-config

            # Must be before (unwrapped) clang in path
            llvmPackages_21.clang-tools

            # Dependencies required to build clang-plugin
            clang_21
            llvmPackages_21.libllvm
            llvmPackages_21.libclang

            gdb

            scip
            protobuf

            pre-commit
          ];

          PODMAN_USERNS = "keep-id";

          AWS_PROFILE = "searchfox";

          shellHook = ''
            echo "TL;DR:"
            echo "- './build-docker.sh' to build the container"
            echo "- './run-docker.sh' to enter the container"
            echo "- 'cd /vagrant; make build-test-repo' inside to build"
            echo "- open http://localhost:16995 in a browser"
          '';
        };

        formatter = pkgs.alejandra;
      }
    )
  );
}
