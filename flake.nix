{
  inputs = {
    nixpkgs = {
      url = "github:nixos/nixpkgs";
    };
    flake-utils = {
      url = "github:numtide/flake-utils";
    };
    fenix = {
      url = "github:nix-community/fenix";
    };
  };
  outputs =
    inputs: inputs.flake-utils.lib.eachDefaultSystem (system:
    let
      pkgs = import inputs.nixpkgs {
        inherit system;
      };
      rust =
        let
          toolchain = {
            channel = "stable";
            date = "2022-02-24";
            sha256 = "sha256-4IUZZWXHBBxcwRuQm9ekOwzc0oNqH/9NkI1ejW7KajU=";
          };
        in
        with inputs.fenix.packages.${system}; combine (with toolchainOf toolchain; [
          cargo
          clippy-preview
          rust-src
          rust-std
          rustc
          rustfmt-preview
          targets.x86_64-unknown-linux-musl.stable.rust-std
        ]);
    in
    rec {
      devShell = pkgs.mkShell {
        packages = with pkgs; [
          rust
        ];

      };
    }
    );
}
