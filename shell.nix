{ pkgs ? import <nixpkgs> {} }:
pkgs.mkShell {
  buildInputs = [ pkgs.stdenv.cc pkgs.lld_12 ];
}
