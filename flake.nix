{
  description = "haccerlink - TUI link program for the TI-84 Plus CE";

  inputs.nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";

  outputs = { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system} = {
        default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc rustfmt clippy ];
        };
        # Shell for building the libticalcs2 oracle used to check that we
        # put the same bytes on the wire as TiLP does.
        oracle = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [ cargo rustc gcc pkg-config ];
          buildInputs = with pkgs; [ libticalcs2 libticables2 libtifiles2 libticonv glib ];
        };
      };

      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "haccerlink";
        version = "0.1.0";
        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;
        meta = with pkgs.lib; {
          description = "TUI link program for the TI-84 Plus CE";
          homepage = "https://github.com/jacks0n9/haccerlink";
          license = licenses.gpl3Plus;
          mainProgram = "haccerlink";
          platforms = platforms.linux;
        };
      };
    };
}
