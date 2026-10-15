{ pkgs, craneLib, source, vulkanLoader }:
let
  common = {
    src = craneLib.cleanCargoSource source;
    strictDeps = true;
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = [ pkgs.fontconfig ];
    cargoExtraArgs = "--locked --bin provlita";
  };
in craneLib.buildPackage (common // {
  cargoArtifacts = craneLib.buildDepsOnly common;
  doCheck = false;
  # The protected process receives no ambient loader configuration. Use the
  # same pinned Mesa/loader closure as Lom, never the host's Vulkan libraries.
  postFixup = ''
    patchelf --add-rpath ${vulkanLoader}/lib $out/bin/provlita
  '';
})
