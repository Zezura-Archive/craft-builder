{
  description = "craft-builder: sync, build and release the craft forks";

  inputs = {
    my-nixpkgs.url = "github:evictedcucumber/nixpkgs";
    nixpkgs.follows = "my-nixpkgs/nixpkgs";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = {
    self,
    nixpkgs,
    my-nixpkgs,
    rust-overlay,
    ...
  }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs {
      inherit system;
      overlays = [
        my-nixpkgs.overlays.default
        rust-overlay.overlays.default
      ];
    };

    rust = pkgs.rust-bin.stable.latest.default.override {
      targets = [
        "x86_64-unknown-linux-gnu"
        "x86_64-pc-windows-msvc"
      ];
    };

    # Tools the builds run; `stdenv.cc` is the host C compiler for build scripts.
    runtimeTools = with pkgs; [
      rust
      cargo-zigbuild
      zig
      cargo-xwin
      llvmPackages.clang-unwrapped
      llvmPackages.lld
      llvmPackages.llvm
      sccache
      git
      pkg-config
      stdenv.cc
    ];

    alsaPkgConfig = "${pkgs.alsa-lib.dev}/lib/pkgconfig";
  in {
    packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
      pname = "craft-builder";
      version = "0.1.0";
      src = self;
      cargoLock.lockFile = ./Cargo.lock;

      nativeBuildInputs = with pkgs; [pkg-config makeWrapper];
      buildInputs = [pkgs.openssl];

      # nixpkgs forces system AWS-LC and libgit2; build the vendored ones.
      preBuild = ''
        unset AWS_LC_SYS_USE_SYSTEM LIBGIT2_NO_VENDOR
      '';

      postInstall = ''
        wrapProgram $out/bin/craft-builder \
          --prefix PATH : ${pkgs.lib.makeBinPath runtimeTools} \
          --suffix PATH : ${pkgs.lib.makeBinPath [pkgs.gh]} \
          --prefix PKG_CONFIG_PATH : ${alsaPkgConfig}
      '';

      meta.mainProgram = "craft-builder";
    };

    apps.${system}.default = {
      type = "app";
      program = "${self.packages.${system}.default}/bin/craft-builder";
    };

    devShells.${system}.default = pkgs.mkShell {
      name = "craft-builder";
      packages =
        runtimeTools
        ++ (with pkgs; [
          rust-analyzer
          openssl
          gh
          just
        ]);
      PKG_CONFIG_PATH = alsaPkgConfig;
    };

    formatter.${system} = pkgs.alejandra;
  };
}
