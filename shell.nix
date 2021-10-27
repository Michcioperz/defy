{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  buildInputs = [ pkgs.stdenv.cc pkgs.pkg-config pkgs.openssl ];
}
