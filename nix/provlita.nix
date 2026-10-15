{ pkgs, craneLib, source, vulkanLoader }:
let
  common = {
    src = pkgs.lib.cleanSourceWith {
      src = source;
      # The renderer embeds this font at compile time.
      filter = path: type:
        craneLib.filterCargoSources path type
        || pkgs.lib.hasPrefix "${source}/assets/fonts/" path;
    };
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
