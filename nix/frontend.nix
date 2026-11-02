# Runtime JavaScript and CSS, packaged without invoking Cargo. The
# manifest keeps bundle order and content digests identical to local builds.
{
  lib,
  python3,
  stdenvNoCC,
}:

stdenvNoCC.mkDerivation {
  pname = "discourse-rs-frontend";
  version = (lib.importTOML ../Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../static
      ../scripts/package-frontend
    ];
  };
  nativeBuildInputs = [ python3 ];
  dontConfigure = true;
  dontBuild = true;
  installPhase = ''
    runHook preInstall
    python3 scripts/package-frontend "$out" --source static
    runHook postInstall
  '';
  meta = {
    description = "Runtime frontend assets for discourse-rs";
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.all;
  };
}
