{
  description = "The niltempus desktop: Sophia, Hagia, Lom, Bemenu and kleis, built as one release";

  # Each component is a flake input that follows its main branch; `nix flake
  # update` moves them and flake.lock records what was built. Build work in
  # progress with `--override-input NAME git+file:///path?ref=BRANCH`.
  #
  # Until the components publish their flakes, the inputs name the local
  # repositories.
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/c59305bab2065cfecc4944690d9eedbb56f3a9fa";
    crane.url = "github:ipetkov/crane";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    sophia = {
      url = "git+file:///home/niltempus/dev/sophia?ref=master";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    hagia = {
      url = "git+file:///home/niltempus/dev/hagia?ref=nix/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    narthex = {
      url = "git+file:///home/niltempus/dev/narthex?ref=nix/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    lom = {
      url = "git+file:///home/niltempus/dev/lom?ref=nix/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    bemenu = {
      url = "git+file:///home/niltempus/src/bemenu?ref=nix/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    kleis = {
      url = "git+file:///home/niltempus/dev/kleis?ref=nix/devshell";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # The Sophia revision Cargo.lock names for the xtask's Sophia crates. It
    # is not yet published, so the GitHub URL in Cargo.lock cannot be
    # fetched. Drop this input once it is.
    sophia-crates = {
      url = "git+file:///home/niltempus/dev/sophia?rev=316d969507cdeb6ef061ea2d9645758113c53db6";
      flake = false;
    };
  };

  outputs = { self, nixpkgs, crane, rust-overlay, sophia, hagia, narthex, lom, bemenu, kleis, sophia-crates }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ (import rust-overlay) ];
      };
      lib = pkgs.lib;
      toolchain = pkgs.rust-bin.stable."1.96.1".minimal;
      craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;

      # Vendor Sophia's crates from the sophia-crates input, which must be
      # exactly the revision Cargo.lock names; every other git dependency is
      # fetched as Cargo.lock says.
      xtaskSrc = craneLib.cleanCargoSource self;
      cargoVendorDir = craneLib.vendorCargoDeps {
        src = xtaskSrc;
        overrideVendorGitCheckout = packages: checkout:
          let
            sophiaPackages = builtins.filter
              (p: lib.hasPrefix "git+https://github.com/sophia-org/sophia.git" p.source)
              packages;
            locked = lib.last (lib.splitString "#" (builtins.head sophiaPackages).source);
          in
          if sophiaPackages == [ ] then checkout
          else
            assert lib.assertMsg (locked == sophia-crates.rev)
              "Cargo.lock names Sophia ${locked}, but sophia-crates is ${sophia-crates.rev}";
            checkout.overrideAttrs (_: { src = sophia-crates; });
      };

      # The session recipe tool (xtask) and the host checker the session runs.
      xtaskArgs = {
        pname = "niltempus-xtask";
        version = "0.1.0";
        src = xtaskSrc;
        inherit cargoVendorDir;
        strictDeps = true;
        cargoExtraArgs = "--locked -p xtask --bins";
        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = [ pkgs.libxkbcommon ];
        doCheck = false;
      };
      xtask = craneLib.buildPackage (xtaskArgs // {
        cargoArtifacts = craneLib.buildDepsOnly xtaskArgs;
      });

      sophiaPackage = sophia.packages.${system}.sophia;
      hagiaPackage = hagia.packages.${system}.default;
      narthexPackage = narthex.packages.${system}.default;
      sdk = "${hagia}/vendor/sophia-desktop-sdk";
      sdkRevision = (builtins.fromJSON (builtins.readFile "${sdk}/manifest.json")).revision;
      revision = input: input.rev or (throw "${input} has uncommitted changes; commit them to build the desktop");

      # The release is named by every locked input's content and commit,
      # which the manifest records: one name is always one release, and any
      # change, even a new commit of the same tree, names a new one. The
      # profile names the components by their installed paths under it.
      releaseId = "niltempus-" + builtins.substring 0 20 (builtins.hashString "sha256"
        (lib.concatMapStringsSep "\n" (input: "${input.narHash} ${revision input}")
          [ self sophia hagia narthex lom bemenu kleis ]));
      installed = "/opt/sophia-niltempus-desktop/releases/${releaseId}/target/release";
      profile = pkgs.replaceVars ./profiles/desktop.kdl {
        hagia = "${installed}/hagia";
        lom = "${installed}/lom";
        bemenu = "${installed}/bemenu-sophia";
        kleis = "${installed}/kleis";
      };
      components = {
        lom = "${lom.packages.${system}.default}/bin/lom";
        bemenu-sophia = "${bemenu.packages.${system}.bemenu-sophia}/bin/bemenu-sophia";
        kleis = "${kleis.packages.${system}.default}/bin/kleis";
      };

      # The release as the session runs it. The build time is the newest
      # input's commit time, so the same lock builds the same release.
      #
      # Before anything is laid out, Sophia checks the profile and Hagia its
      # policy, as at login. The check reads a copy that names the store
      # binaries instead of the installed paths, and an empty file for each
      # component's private config: those live in the user's home, which the
      # build cannot read, and only their presence is checked here.
      desktop = pkgs.runCommand "niltempus-desktop-${releaseId}" {
        # git hashes the vendored C SDK snapshot to check its revision.
        nativeBuildInputs = [ pkgs.bash pkgs.coreutils pkgs.gnused pkgs.gawk pkgs.gnugrep pkgs.git ];
        passthru = { inherit releaseId; };
      } ''
        # The launcher checks Hagia's WM environment contract at every login;
        # check the packaged Hagia against the same line now.
        hagia=${hagiaPackage}/bin/hagia
        expected=$(sed -n "s/^expected='\(.*\)'$/\1/p" ${self}/tools/installed/sophia-niltempus-desktop-session)
        contract=$($hagia config check-environment-contract)
        test -n "$expected" && test "$contract" = "$expected" \
          || { echo "Hagia does not confirm the WM environment contract: $contract" >&2; exit 1; }

        mkdir check
        : > check/private-config
        printf '#!${pkgs.bash}/bin/sh\nset -eu\ntest "$#" = 1\nexec %s config check --config="$1"\n' "$hagia" > check/policy-checker
        chmod 0755 check/policy-checker
        sed -E \
          -e "s|${installed}/hagia|$hagia|" \
          -e "s|${installed}/lom|${components.lom}|" \
          -e "s|${installed}/bemenu-sophia|${components.bemenu-sophia}|" \
          -e "s|${installed}/kleis|${components.kleis}|g" \
          -e "s|^([[:space:]]*config )\"[^\"]*\"|\1\"$PWD/check/private-config\"|" \
          ${profile} > check/desktop.kdl
        if grep -q "${installed}" check/desktop.kdl; then
          echo "the profile names an installed path the check does not cover" >&2
          exit 1
        fi
        ${sophiaPackage}/bin/sophia config check-session-profile \
          --desktop-profile=$PWD/check/desktop.kdl --default-wm=$hagia \
          --policy-checker=$PWD/check/policy-checker | tee check/preflight.log
        test "$(grep '^sophia_session_profile_preflight ' check/preflight.log)" \
          = "sophia_session_profile_preflight schema=1 status=accepted policy=validated"

        built=$(date -u -d @${toString self.lastModified} +%Y-%m-%dT%H:%M:%SZ)
        ${xtask}/bin/xtask assemble-nix \
          --out=$out --built-at-utc=$built --release-id=${releaseId} \
          --repo=${self} --integration-commit=${revision self} \
          --sophia-tree=${sophia} --sophia-rev=${revision sophia} \
          --sophia=${sophiaPackage}/bin/sophia \
          --factotum=${sophiaPackage}/bin/sophia-factotum \
          --pam-helper=${sophiaPackage}/bin/sophia-factotum-pam \
          --xtask=${xtask}/bin/xtask --preflight=${xtask}/bin/active-session-preflight \
          --hagia=$hagia --hagia-commit=${revision hagia} \
          --narthex=${narthexPackage}/bin/narthex --narthex-commit=${revision narthex} \
          --profile=${hagia}/examples/config/default.kdl \
          --c-sdk=${sdk} --c-sdk-rev=${sdkRevision} \
          --verifier-interpreter=${pkgs.bash}/bin/bash \
          --file=target/release/lom=${components.lom} \
          --file=target/release/bemenu-sophia=${components.bemenu-sophia} \
          --file=target/release/kleis=${components.kleis} \
          --file=share/sophia-niltempus-desktop/desktop.kdl=${profile}
      '';
    in
    {
      packages.${system} = {
        inherit xtask desktop;
        default = desktop;
      };
    };
}
