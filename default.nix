{ pkgs ? import <nixpkgs> {} }:

let
  spider = pkgs.rustPlatform.buildRustPackage {
    pname = "spider";
    version = "2.36.54";

    src = ./.;

    cargoLock = {
      lockFile = ./Cargo.lock;
    };

    nativeBuildInputs = [];

    meta = with pkgs.lib; {
      description = "A web crawler and scraper, building blocks for data curation workloads.";
      homepage = "https://github.com/spider-rs/spider";
      license = licenses.mit;
      maintainers = with maintainers; [ j-mendez ];
      platforms = platforms.all;
    };
  };
in
{
  inherit spider;
}
