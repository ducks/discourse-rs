# What DISCOURSE_SRC (a Discourse checkout with its bundle installed)
# supplies at runtime, laid out as src/config.rs looks for it:
#
#   public/images/                       stock images (logos, avatar.png, ...)
#   app/assets/images/push-notifications/ the push notifications' icons
#   .bundle/ruby/3.4.0/gems/discourse-fonts-<ver>/vendor/assets/fonts/
#   .bundle/ruby/3.4.0/gems/discourse-emojis-<ver>/dist/emoji/
#
# The images come from Discourse at the vendored commit
# (vendor/discourse/DISCOURSE_REF), fetched sparsely; the gems from
# rubygems.org at the versions that commit's Gemfile.lock pins, with its
# checksums. deploy/install-emoji installs the same discourse-emojis gem.
{
  lib,
  stdenvNoCC,
  fetchFromGitHub,
  fetchurl,
}:

let
  ref = builtins.readFile ../vendor/discourse/DISCOURSE_REF;
  field =
    name:
    let
      m = builtins.match ".*(^|\n)${name}=([^\n]*).*" ref;
    in
    if m == null then throw "DISCOURSE_REF has no ${name}" else builtins.elemAt m 1;
  commit = field "commit";

  # discourse-emojis: DISCOURSE_REF's version, deploy/install-emoji's
  # checksum (Gemfile.lock's at that commit).
  emojisVersion = field "discourse_emojis";
  emojisGem = fetchurl {
    url = "https://rubygems.org/downloads/discourse-emojis-${emojisVersion}.gem";
    sha256 = "e986526ec835eb8de1640da6a853767272a3fc15b756666f072ab95103f3c93c";
  };

  # discourse-fonts: src/stylesheet/fonts.rs's VERSION, the font urls'.
  fontsVersion =
    let
      m = builtins.match ''.*pub const VERSION: &str = "([^"]+)";.*'' (
        builtins.readFile ../src/stylesheet/fonts.rs
      );
    in
    builtins.head m;
  fontsGem = fetchurl {
    url = "https://rubygems.org/downloads/discourse-fonts-${fontsVersion}.gem";
    sha256 = "78d4ddd671615908303a675427039d8d787c935e6deae184c6e143c18c6e0033";
  };

  discourse = fetchFromGitHub {
    owner = "discourse";
    repo = "discourse";
    rev = commit;
    sparseCheckout = [
      "public/images"
      "app/assets/images/push-notifications"
    ];
    hash = "sha256-AdRlEpuRbQK5wNgJAjMsxilCsp2m2ccs/Q3pleu8RHY=";
  };
in
stdenvNoCC.mkDerivation {
  pname = "discourse-assets";
  version = "${emojisVersion}-${fontsVersion}-${builtins.substring 0 11 commit}";

  dontUnpack = true;
  # Data only: nothing for fixup to patch, and scanning the emoji is slow.
  dontFixup = true;

  installPhase = ''
    runHook preInstall

    mkdir -p $out/public $out/app/assets/images
    cp -R ${discourse}/public/images $out/public/images
    cp -R ${discourse}/app/assets/images/push-notifications $out/app/assets/images/

    gems=$out/.bundle/ruby/3.4.0/gems
    unpack_gem() { # <gem> <name-version> <path in the gem>
      mkdir -p "$gems/$2"
      tar -xOf "$1" data.tar.gz \
        | tar -xz --no-same-owner --no-same-permissions -C "$gems/$2" "$3"
    }
    unpack_gem ${fontsGem} discourse-fonts-${fontsVersion} vendor/assets/fonts
    unpack_gem ${emojisGem} discourse-emojis-${emojisVersion} dist/emoji

    # What src/config.rs and the VM test rely on.
    test -f $out/public/images/discourse-logo-sketch.png
    test -f $gems/discourse-fonts-${fontsVersion}/vendor/assets/fonts/Roboto-Regular.woff2
    test -f $gems/discourse-emojis-${emojisVersion}/dist/emoji/twitter/heart.png

    runHook postInstall
  '';

  passthru = {
    inherit commit emojisVersion fontsVersion;
  };

  meta = {
    description = "Discourse's stock images, fonts and emoji, as DISCOURSE_SRC for discourse-rs";
    license = lib.licenses.gpl2Plus;
    platforms = lib.platforms.all;
  };
}
